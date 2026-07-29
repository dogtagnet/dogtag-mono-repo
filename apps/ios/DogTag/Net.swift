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

    /// Why a requested endpoint could not be used. A wrong-chain answer is kept separate so the
    /// settings screen can say exactly what happened; neither it nor a transport/malformed-response
    /// failure is allowed to reach an address-bound chain read.
    enum EndpointFailure: Equatable {
        case invalidURL
        case unavailable
        case invalidChainIdResponse
        case wrongChain(reported: UInt64)
    }

    /// The route chosen after probing `eth_chainId`. `unavailable` means even the bundled endpoint
    /// failed the guard, so no contract read may be sent or trusted.
    enum EndpointRoute: Equatable {
        case bundled
        case custom(String)
        case bundledFallback(EndpointFailure)
        case unavailable(custom: EndpointFailure?, bundled: EndpointFailure)

        var rpcUrl: String? {
            switch self {
            case .bundled, .bundledFallback: return AppConfig.roaxRpc
            case .custom(let url): return url
            case .unavailable: return nil
            }
        }
    }

    /// Check the requested peer immediately before a read. A custom peer that is unavailable,
    /// malformed, or reports a chain other than the one whose contract addresses are bundled falls
    /// back to the bundled peer. The bundled peer is guarded too: if it cannot establish the bundled
    /// chain id, the caller gets UNKNOWN rather than an answer from an unestablished chain.
    static func endpointRoute(rpcUrl: String) async -> EndpointRoute {
        await endpointRoute(
            rpcUrl: rpcUrl,
            expectedChainId: RoaxConfig.load().chainId,
            probe: { url, body in await Http.postJSON(url, body: body) }
        )
    }

    /// Injectable seam for the host-safe endpoint-selection tests. The probe receives the complete
    /// JSON-RPC body so tests can also pin that the guard asks `eth_chainId`, not `net_version` or a
    /// contract whose address is already chain-specific.
    static func endpointRoute(
        rpcUrl: String,
        expectedChainId: Int,
        probe: (String, String) async -> Http.Response
    ) async -> EndpointRoute {
        let bundled = AppConfig.roaxRpc
        let requested = RpcEndpointSettings.normalizedURL(rpcUrl)

        if requested == bundled {
            if let failure = await chainFailure(
                url: bundled, expectedChainId: expectedChainId, probe: probe) {
                return .unavailable(custom: nil, bundled: failure)
            }
            return .bundled
        }

        let customFailure: EndpointFailure
        if let requested {
            if let failure = await chainFailure(
                url: requested, expectedChainId: expectedChainId, probe: probe) {
                customFailure = failure
            } else {
                return .custom(requested)
            }
        } else {
            customFailure = .invalidURL
        }

        if let bundledFailure = await chainFailure(
            url: bundled, expectedChainId: expectedChainId, probe: probe) {
            return .unavailable(custom: customFailure, bundled: bundledFailure)
        }
        return .bundledFallback(customFailure)
    }

    private static func chainFailure(
        url: String,
        expectedChainId: Int,
        probe: (String, String) async -> Http.Response
    ) async -> EndpointFailure? {
        let payload: [String: Any] = [
            "jsonrpc": "2.0", "id": 1, "method": "eth_chainId", "params": [],
        ]
        guard let raw = try? JSONSerialization.data(withJSONObject: payload),
              let body = String(data: raw, encoding: .utf8) else {
            return .invalidChainIdResponse
        }
        let response = await probe(url, body)
        guard response.ok else { return .unavailable }
        guard let data = response.body.data(using: .utf8),
              let object = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any],
              object["error"] == nil,
              let result = object["result"] as? String,
              result.hasPrefix("0x"),
              result.count > 2,
              let reported = UInt64(result.dropFirst(2), radix: 16) else {
            return .invalidChainIdResponse
        }
        guard reported == UInt64(expectedChainId) else { return .wrongChain(reported: reported) }
        return nil
    }

    /// The single transport seam for all blockchain JSON-RPC reads below. A custom peer that passes
    /// the chain guard but disappears before the actual read gets one guarded retry on the bundled
    /// endpoint. Nothing here applies to central/provider or QR-discovered role APIs.
    private static func guardedPostJSON(rpcUrl: String, body: String) async -> Http.Response {
        await guardedPostJSON(
            rpcUrl: rpcUrl,
            body: body,
            expectedChainId: RoaxConfig.load().chainId,
            probe: { url, probeBody in await Http.postJSON(url, body: probeBody) },
            request: { url, requestBody in await Http.postJSON(url, body: requestBody) }
        )
    }

    /// Injectable form of the production transport seam. Keeping chain probes and address-bound
    /// requests as separate closures lets tests prove that a rejected peer receives no contract call
    /// and that a retry cannot bypass a fresh guard on the bundled peer.
    static func guardedPostJSON(
        rpcUrl: String,
        body: String,
        expectedChainId: Int,
        probe: (String, String) async -> Http.Response,
        request: (String, String) async -> Http.Response
    ) async -> Http.Response {
        let route = await endpointRoute(
            rpcUrl: rpcUrl,
            expectedChainId: expectedChainId,
            probe: probe
        )
        guard let selected = route.rpcUrl else {
            return Http.Response(code: -1, body: "chain guard could not establish the bundled chain")
        }

        let response = await request(selected, body)
        guard case .custom = route, !response.ok else { return response }

        // The custom peer passed `eth_chainId` but then became unavailable. Validate the fallback
        // endpoint immediately too; never turn a transport retry into an unguarded read.
        let fallback = await endpointRoute(
            rpcUrl: AppConfig.roaxRpc,
            expectedChainId: expectedChainId,
            probe: probe
        )
        guard let bundled = fallback.rpcUrl else {
            return Http.Response(code: -1, body: "chain guard could not establish the bundled chain")
        }
        return await request(bundled, body)
    }

    /// The outcome of a read whose on-chain answer is an address or a word.
    ///
    /// Three outcomes, never two: `unset` is the chain ANSWERING with its zero value (nobody ever
    /// wrote that slot), while `unresolved` is the read not answering at all - a transport failure, a
    /// revert, or a reply too short to hold the value. Collapsing the pair into one `nil` is how a
    /// dropped connection came to be reported to the holder as the definite chain fact "no factory
    /// clone ever issued this root". Both remain indeterminate and neither can ever become a pass;
    /// only what the owner is told differs.
    enum HexRead {
        case found(String)
        case unset
        case unresolved(String)
    }

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
        let resp = await guardedPostJSON(rpcUrl: rpcUrl, body: bodyStr)
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

    // Derived, not hard-coded — same reasoning as `isValidSelector` above. These three were previously
    // string constants whose comments claimed to be these hashes; the values happened to be right, but
    // `consumed()` is the owner-hidden completion signal and `isWhitelistedFor()` is the groomer
    // authorization gate, so a silent drift would land on two safety gates.
    private static let isWhitelistedForSelector = functionSelector("isWhitelistedFor(bytes32,address)")
    private static let consumedSelector = functionSelector("consumed(bytes32)")
    private static let profileRootSelector = functionSelector("profileRoot(uint256)")

    // ---- the issuer↔domain binding chain -------------------------------------------------------
    private static let isCloneSelector = functionSelector("isClone(address)")
    private static let domainOfSelector = functionSelector("domainOf(address)")
    private static let issuerNameSelector = functionSelector("name()")

    /// Reading a dynamic `string` return has THREE outcomes, and collapsing any two of them is how a
    /// fail-open gets reintroduced:
    ///   - `.value` — the call returned ABI-encoded bytes (possibly an empty string, which is a real
    ///     answer meaning "no claim published").
    ///   - `.noContract` — an EMPTY `eth_call` result. That is what a call to an address with no code
    ///     returns, i.e. the registry is not deployed for this config. It is emphatically NOT an empty
    ///     string, and must surface as "we could not check", never as "no claim".
    ///   - `.failure` — the RPC did not answer.
    enum StringRead {
        case value(String)
        case noContract
        case failure(String)
    }

    /// Reading an `address` return has the same three outcomes, kept apart for the same reason:
    ///   - `.value` — a real, non-zero address.
    ///   - `.noRecord` — the mapping answered with the zero address, i.e. "no entry for this key". A
    ///     definite answer, not a failure.
    ///   - `.failure` — the RPC did not answer, or the body was not an address word (an empty result,
    ///     which is what a call to an address with no code returns, lands here). Emphatically NOT
    ///     "no record": a read we could not make is evidence of nothing.
    enum AddressRead {
        case value(String)
        case noRecord
        case failure(String)
    }

    /// `DogTagIssuerFactory.rootIssuer(root)` — the clone that ISSUED this root, write-once on-chain.
    ///
    /// THE authoritative answer to "which contract issued this credential". The document's
    /// `issuer.documentStore` is only a claim, and pointing it at ANOTHER authority's genuine clone is
    /// the sharper form of the relabelling attack: link 1 (`isClone`) passes because the target really is
    /// a factory clone, so without this read the phone renders that other authority's on-chain identity.
    static func rootIssuer(
        rpcUrl: String, factory: String, root: String, atBlock: UInt64?
    ) async -> AddressRead {
        guard !factory.isEmpty, !root.isEmpty else { return .failure("missing factory/root") }
        let data = rootIssuerSelector + pad32(root)
        switch await ethCall(rpcUrl: rpcUrl, to: factory, data: data, atBlock: atBlock) {
        case .failure(let e): return .failure(e)
        case .success(let hex):
            guard let addr = decodeAbiAddress(hex) else { return .failure("not an address word") }
            return isZeroAddress(addr) ? .noRecord : .value(addr)
        }
    }

    /// Decode a right-aligned 32-byte `address` word to lowercase `0x..`. Returns nil rather than
    /// guessing for anything that is not one — including an EMPTY result (no contract at that address)
    /// and a word with dirty high bytes.
    static func decodeAbiAddress(_ hex: String) -> String? {
        let h = (hex.hasPrefix("0x") ? String(hex.dropFirst(2)) : hex).lowercased()
        guard h.count == 64, h.allSatisfy({ $0.isASCII && $0.isHexDigit }) else { return nil }
        guard h.prefix(24).allSatisfy({ $0 == "0" }) else { return nil }
        return "0x" + h.suffix(40)
    }

    /// The zero address, i.e. an unset mapping slot.
    static func isZeroAddress(_ addr: String) -> Bool {
        let h = addr.hasPrefix("0x") ? String(addr.dropFirst(2)) : addr
        return !h.isEmpty && h.allSatisfy { $0 == "0" }
    }

    /// The current chain head, so every read in one verification can be pinned to ONE block. Against a
    /// world where DNS changes and clones are superseded, an unanchored answer is not auditable.
    static func blockNumber(rpcUrl: String) async -> UInt64? {
        let payload: [String: Any] = ["jsonrpc": "2.0", "id": 1, "method": "eth_blockNumber", "params": []]
        guard let raw = try? JSONSerialization.data(withJSONObject: payload),
              let bodyStr = String(data: raw, encoding: .utf8) else { return nil }
        let resp = await guardedPostJSON(rpcUrl: rpcUrl, body: bodyStr)
        guard resp.ok, let d = resp.body.data(using: .utf8),
              let o = (try? JSONSerialization.jsonObject(with: d)) as? [String: Any],
              let hex = o["result"] as? String else { return nil }
        return UInt64(hex.hasPrefix("0x") ? String(hex.dropFirst(2)) : hex, radix: 16)
    }

    /// `DogTagIssuerFactory.isClone(candidate)` — LINK 1. Proves the contract came from the DogTag
    /// factory, i.e. passed through KYC-gated `createIssuer`. Without it a domain binding shows only
    /// that whoever deployed some contract also controls a domain.
    static func isClone(rpcUrl: String, factory: String, candidate: String, atBlock: UInt64?) async -> Result {
        guard !factory.isEmpty, !candidate.isEmpty else { return .unknown("missing factory/candidate") }
        let data = isCloneSelector + padAddr(candidate)
        switch await ethCall(rpcUrl: rpcUrl, to: factory, data: data, atBlock: atBlock) {
        case .failure(let e): return .unknown(e)
        case .success(let hex):
            // A call to a non-contract returns empty. That is not `false` — it is "no answer".
            if hex.isEmpty { return .unknown("empty result") }
            return hex.contains { $0 != "0" } ? .valid : .invalid
        }
    }

    /// `IssuerDomainRegistry.domainOf(clone)` — LINK 2. The domain the CONTRACT claims. Never the
    /// document's `issuer.domain`, which is outside the Merkle root and can be relabelled at will.
    static func issuerClaimedDomain(
        rpcUrl: String, domainRegistry: String, clone: String, atBlock: UInt64?
    ) async -> StringRead {
        guard !domainRegistry.isEmpty, !clone.isEmpty else { return .failure("missing registry/clone") }
        let data = domainOfSelector + padAddr(clone)
        return await ethCallString(rpcUrl: rpcUrl, to: domainRegistry, data: data, atBlock: atBlock)
    }

    /// `DogTagIssuer.name()` — the clone's own name, written by the factory's `onlyOwner` `createIssuer`
    /// at KYC time. The only issuer name a surface may present: the document's `issuer.name` is outside
    /// the Merkle root, so relabelling it alone passes integrity AND the `data.issuer` DID check (the DID
    /// carries a domain, not a name).
    static func issuerOnchainName(rpcUrl: String, clone: String, atBlock: UInt64?) async -> StringRead {
        guard !clone.isEmpty else { return .failure("missing clone") }
        return await ethCallString(rpcUrl: rpcUrl, to: clone, data: issuerNameSelector, atBlock: atBlock)
    }

    /// Decode a single ABI-encoded dynamic `string` return: [32-byte offset][32-byte length][bytes].
    private static func ethCallString(
        rpcUrl: String, to: String, data: String, atBlock: UInt64?
    ) async -> StringRead {
        switch await ethCall(rpcUrl: rpcUrl, to: to, data: data, atBlock: atBlock) {
        case .failure(let e): return .failure(e)
        case .success(let hex):
            if hex.isEmpty { return .noContract }
            return .value(decodeAbiString(hex) ?? "")
        }
    }

    /// Decode `[offset][len][bytes...]`. Returns nil on a malformed body rather than guessing.
    static func decodeAbiString(_ hex: String) -> String? {
        let bytes = hexToBytes(hex)
        guard bytes.count >= 64 else { return nil }
        // The offset is a byte offset from the start of the return data; for a single string it is 0x20,
        // but read it rather than assume so a tuple-wrapped return still decodes.
        guard let offset = beInt(bytes[0..<32]), offset + 32 <= bytes.count else { return nil }
        guard let len = beInt(bytes[offset..<(offset + 32)]) else { return nil }
        let start = offset + 32
        guard start + len <= bytes.count else { return nil }
        return String(bytes: bytes[start..<(start + len)], encoding: .utf8)
    }

    private static func hexToBytes(_ hex: String) -> [UInt8] {
        let h = hex.hasPrefix("0x") ? String(hex.dropFirst(2)) : hex
        var out: [UInt8] = []
        out.reserveCapacity(h.count / 2)
        var idx = h.startIndex
        while idx < h.endIndex, h.index(after: idx) < h.endIndex {
            let next = h.index(idx, offsetBy: 2)
            if let b = UInt8(h[idx..<next], radix: 16) { out.append(b) } else { return out }
            idx = next
        }
        return out
    }

    /// Big-endian read of a 32-byte word as an `Int`. Returns nil when anything is set above the low 4
    /// bytes, or when the result would not fit a 32-bit `Int` — a length or offset that large is not a
    /// value we will honour.
    ///
    /// Returning an OPTIONAL rather than a sentinel is load-bearing: the caller used to write
    /// `Int(beUInt(...))`, and `Int.init(_: UInt64)` TRAPS on overflow, so an adversarial word (the
    /// `UInt64.max` reject sentinel, or any value above `Int.max`) crashed the app instead of rejecting
    /// the body — the exact opposite of this decoder's "returns nil rather than guessing" contract, and
    /// reachable from any `eth_call` reply while a credential sheet is open. Mirror of the Kotlin
    /// `RoaxRpc.beInt`, which was already correct; keep the two in step.
    private static func beInt(_ slice: ArraySlice<UInt8>) -> Int? {
        let b = Array(slice)
        guard b.count == 32 else { return nil }
        if b.prefix(28).contains(where: { $0 != 0 }) { return nil }
        var v: UInt64 = 0
        for byte in b.suffix(4) { v = (v << 8) | UInt64(byte) }
        guard v <= UInt64(Int32.max) else { return nil }
        return Int(v)
    }

    /// `DogTagSBTConsent.profileRoot(dogTagId)` → the on-chain DOG_PROFILE root (0x.. 32-byte), or
    /// nil. `dogTagId` is the CANONICAL field-hashed id (`dogTagIdFieldHex`), the same value the tag
    /// was minted under — NOT the raw decimal handle, which reads an unset slot forever. This is the
    /// SBT anchor used to confirm an issued DOG_PROFILE (NOT the DogTagIssuer-clone isValid).
    static func profileRoot(rpcUrl: String, dogTagSbt: String, dogTagId: String) async -> String? {
        guard !dogTagSbt.isEmpty, !dogTagId.isEmpty else { return nil }
        let data = profileRootSelector + padUint(dogTagId)
        switch await ethCall(rpcUrl: rpcUrl, to: dogTagSbt, data: data) {
        case let .success(hex):
            guard hex.count == 64, hex.allSatisfy({ $0.isHexDigit }) else { return nil }
            return "0x" + hex.lowercased()
        case .failure: return nil
        }
    }

    /// `IssuerRegistry.isWhitelistedFor(key, signer)` — the PRE-PROOF groomer check. On Unknown the
    /// caller MUST hard-stop (this gate is a user-safety requirement → unknown == not authorized).
    ///
    /// A reply too short to hold a bool word is the registry NOT ANSWERING (a codeless or wrong
    /// address returns `0x` as a SUCCESSFUL call), so it is `.unknown` - never the definite `.invalid`
    /// that accuses a genuine credential's issuer of being unauthorised. Only a full 32-byte zero word
    /// is a real "no".
    static func isWhitelistedFor(rpcUrl: String, issuerRegistry: String, key: String, signer: String) async -> Result {
        guard !issuerRegistry.isEmpty, !key.isEmpty, !signer.isEmpty else { return .unknown("missing addr/key/signer") }
        let data = isWhitelistedForSelector + pad32(key) + padAddr(signer)
        switch await ethCall(rpcUrl: rpcUrl, to: issuerRegistry, data: data) {
        case let .success(hex):
            guard hex.count >= 64 else { return .unknown("the registry returned no answer") }
            let truthy = hex.contains { $0 != "0" }
            return truthy ? .valid : .invalid
        case let .failure(reason):
            return .unknown(reason)
        }
    }

    /// `DogTagIssuer.issuedBy(root)` → the H-1 originator that actually called `issue(root)` on this
    /// clone. `.unset` when the clone never issued it (the on-chain zero address), `.unresolved` when
    /// the read did not answer. Selector DERIVED from the signature, never a constant (see
    /// `isValidSelector`).
    static func issuedBy(rpcUrl: String, documentStore: String, root: String) async -> HexRead {
        guard !documentStore.isEmpty, !root.isEmpty else { return .unresolved("missing addr/root") }
        let data = issuedBySelector + pad32(root)
        switch await ethCall(rpcUrl: rpcUrl, to: documentStore, data: data) {
        case let .success(hex):
            // address is right-aligned in a 32-byte word; all-zero == never issued here.
            guard hex.count >= 40 else { return .unresolved("the issuer clone returned no address") }
            guard hex.contains(where: { $0 != "0" }) else { return .unset }
            return .found("0x" + hex.suffix(40).lowercased())
        case let .failure(reason): return .unresolved(reason)
        }
    }

    private static let issuedBySelector = functionSelector("issuedBy(bytes32)")
    private static let rootIssuerSelector = functionSelector("rootIssuer(bytes32)")
    private static let recordTypeSelector = functionSelector("recordType()")


    /// `DogTagIssuer.recordType()` → the clone's own immutable record-type key. `.unset` when the
    /// contract reports the zero word (uninitialized / not a clone), `.unresolved` when the read did
    /// not answer.
    static func recordTypeOf(rpcUrl: String, issuerClone: String) async -> HexRead {
        guard !issuerClone.isEmpty else { return .unresolved("missing issuer clone") }
        switch await ethCall(rpcUrl: rpcUrl, to: issuerClone, data: recordTypeSelector) {
        case let .success(hex):
            guard hex.count == 64, hex.allSatisfy({ $0.isHexDigit }) else {
                return .unresolved("the issuer clone returned no record-type word")
            }
            guard hex.contains(where: { $0 != "0" }) else { return .unset }
            return .found("0x" + hex.lowercased())
        case let .failure(reason): return .unresolved(reason)
        }
    }

    /// The ISSUER-WHITELIST pillar for a held credential, resolved end-to-end on-chain.
    ///
    /// The document's `issuer` block is NOT covered by the Merkle root, so `name`, `domain`,
    /// `recordType` and - the sharp one - `documentStore` are chosen by whoever built the document.
    /// Asking `documentStore` whether the credential is valid, and who issued it, is asking the
    /// suspect for their own references: deploy a contract that answers `isValid = true` and names a
    /// genuinely whitelisted signer, and integrity plus the issuance read both still pass.
    ///
    /// So the clone is resolved from the FACTORY in the app's own bundled `roax.json`
    /// (`rootIssuer`) - never from the document. Then the record type comes from that clone's own
    /// `recordType()`, and the issuing signer from its `issuedBy`, checked against the app's own
    /// `IssuerRegistry`. An envelope naming a different clone, or a different record type, than the
    /// chain does is a definite `.invalid`, not merely unresolved.
    ///
    /// `.unknown` means the pillar did not resolve; a caller must treat that as indeterminate, never
    /// as a pass. A read that FAILED and a slot the chain says is empty both land there, but they say
    /// so differently: only the latter may state a chain fact.
    ///
    /// A document naming no issuer contract at all is indeterminate too, not a mismatch. There is
    /// nothing to compare the factory's answer against, and the DOG_PROFILE records that legitimately
    /// carry no `documentStore` anchor in the SBT instead - so a comparison against the empty string
    /// would call every one of them a forgery.
    static func issuerWhitelistPillar(
        rpcUrl: String, issuerRegistry: String, issuerFactory: String,
        documentStore: String, root: String, recordType: String
    ) async -> Result {
        guard !issuerRegistry.isEmpty else { return .unknown("no IssuerRegistry configured") }
        guard !issuerFactory.isEmpty else { return .unknown("no DogTagIssuerFactory configured") }
        guard !recordType.isEmpty else { return .unknown("document declares no recordType") }
        let claimedStore = documentStore.trimmingCharacters(in: .whitespaces)
        guard !claimedStore.isEmpty else { return .unknown("document names no issuer contract") }
        // Uses the SAME factory read the issuer-domain binding uses, so both features agree on which
        // contract issued a root. `.noRecord` (the factory answered, and has none) and `.failure` (we
        // could not ask) stay distinct: neither is a pass, but only one is evidence.
        let clone: String
        switch await rootIssuer(rpcUrl: rpcUrl, factory: issuerFactory, root: root, atBlock: nil) {
        case let .value(addr): clone = addr
        case .noRecord: return .unknown("no factory clone ever issued this root")
        case let .failure(r): return .unknown("could not read the factory's issuer index (\(r))")
        }
        // The envelope points somewhere other than the contract that actually issued the root: a
        // definite misrepresentation, refused before the registry is consulted.
        guard clone.caseInsensitiveCompare(claimedStore) == .orderedSame else {
            return .invalid
        }
        let chainRecordType: String
        switch await recordTypeOf(rpcUrl: rpcUrl, issuerClone: clone) {
        case let .found(key): chainRecordType = key
        case .unset: return .unknown("issuer clone reports no recordType")
        case let .unresolved(r): return .unknown("could not read the issuer clone's record type (\(r))")
        }
        guard chainRecordType.caseInsensitiveCompare(recordTypeKey(recordType)) == .orderedSame else {
            return .invalid
        }
        let signer: String
        switch await issuedBy(rpcUrl: rpcUrl, documentStore: clone, root: root) {
        case let .found(addr): signer = addr
        case .unset: return .unknown("issuer clone reports no issuer for this root")
        case let .unresolved(r): return .unknown("could not read who issued this root (\(r))")
        }
        return await isWhitelistedFor(
            rpcUrl: rpcUrl, issuerRegistry: issuerRegistry,
            key: chainRecordType, signer: signer)
    }

    /// `keccak256(recordType utf8)` — the `IssuerRegistry` whitelist key, and the same value the
    /// clone's own `recordType()` holds. Mirrors the backend `record_type_key` and the web
    /// `recordTypeKey`; verified against `cast keccak "TRAVEL_CLEARANCE"` on chain 135.
    static func recordTypeKey(_ recordType: String) -> String {
        "0x" + Keccak256.digest(Data(recordType.utf8)).map { String(format: "%02x", $0) }.joined()
    }

    /// `VerificationRegistry.consumed(nullifier)` → true once the relayer's `recordVerificationZK`
    /// (or the legacy path) has landed on-chain for this nullifier. This is the CANONICAL completion
    /// signal for the async export/verify flow: the groomer host records in the background, so the
    /// phone polls this until it flips true. `nullifier` is the consent proof's index-3 public signal;
    /// callers resolve it through `PublicSignalIndex` rather than hard-coding it (index 4 is `R`,
    /// which is never marked consumed). Accepts a decimal field
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

    private static let getContractSetSelector = functionSelector("getContractSet(bytes32)")
    private static let getActiveArtifactSetSelector = functionSelector("getActiveArtifactSet(bytes32)")

    /// keccak256 of a version string as a 32-byte word — the `ProtocolRegistry` map key
    /// (`contractSetId`) for that version. `AnchorResolver.protocolVersion` is the canonical key.
    static func versionId(_ version: String) -> String {
        Keccak256.digest(Data(version.utf8)).map { String(format: "%02x", $0) }.joined()
    }

    /// `ProtocolRegistry.getContractSet(versionId)` → the on-chain contract-set record (M-4 PR3).
    /// Returns nil when the registry is unconfigured/unreachable OR the version is unpublished (the
    /// getter reverts "unknown contract set"), so verification fails closed. `protocolRegistry`
    /// empty (not yet deployed) is a nil, by design.
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
    /// here (unconfigured registry, unpublished version, or no binding) fails verification closed.
    /// Decodes only `artifactSetId`/`minAppVersion`/`active` — see `AnchorResolver`.
    static func getActiveArtifactSet(rpcUrl: String, protocolRegistry: String, version: String) async -> AnchorResolver.ArtifactSetRecord? {
        guard !protocolRegistry.isEmpty else { return nil }
        let data = getActiveArtifactSetSelector + versionId(version)
        switch await ethCall(rpcUrl: rpcUrl, to: protocolRegistry, data: data) {
        case let .success(hex): return AnchorResolver.decodeArtifactSet(hex)
        case .failure: return nil
        }
    }

    private enum CallResult { case success(String); case failure(String) }

    private static func ethCall(
        rpcUrl: String, to: String, data: String, atBlock: UInt64? = nil
    ) async -> CallResult {
        // Pin to a block when we have one so a whole verification is a consistent, reproducible
        // snapshot; `latest` otherwise, and the caller reports the answer as unanchored.
        let blockTag = atBlock.map { "0x" + String($0, radix: 16) } ?? "latest"
        let payload: [String: Any] = [
            "jsonrpc": "2.0", "id": 1, "method": "eth_call",
            "params": [["to": to, "data": data], blockTag],
        ]
        guard let raw = try? JSONSerialization.data(withJSONObject: payload),
              let bodyStr = String(data: raw, encoding: .utf8) else { return .failure("encode") }
        let resp = await guardedPostJSON(rpcUrl: rpcUrl, body: bodyStr)
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
    /// Human identity collected by the issuing vet, shown to the owner before binding. The values
    /// the device actually FOLDS into `R` come from `identityLeaves` (below), which carry the
    /// vet-generated salts.
    struct OwnerIdentity {
        let countryOfIdentification: String
        let identification: String
        let name: String
    }

    /// D1: one vet-salted identity attribute leaf the device MUST fold into `R` alongside the pet
    /// attributes. `{keyPath, saltHex, tag, value}` - the salt is the VET's (at bind it requires
    /// the posted identity openings to EXACTLY match its own stored set while rebuilding `R` from
    /// the full leaf list, refusing the mint otherwise), and the device persists the triple as a
    /// disclosure opening.
    struct IdentityLeaf {
        let keyPath: String
        let saltHex: String
        let tag: UInt8
        let value: String
    }

    struct IssueMicrochip {
        let code: String
        let standard: String
        let implantDate: String
        let bodyLocation: String
    }

    struct IssueWeight {
        let unit: String
        let value: String
        let measuredOn: String
    }

    /// Pet metadata the device commits as ordinary, random-salted profile attribute leaves.
    struct IssuePet {
        let name: String
        let species: String
        let breedVbo: String
        let breedLabel: String
        let sex: String
        let neuterStatus: String
        let dateOfBirth: String
        let weightHistory: [IssueWeight]
        let microchip: IssueMicrochip

        /// The intentionally narrow v1 pet profile. The three reserved owner-control leaves are added
        /// inside the Rust builder; vet-held human identity is a later slice and is excluded here.
        var profileAttributeValues: [(keyPath: String, value: String, tag: UInt8)] {
            var values: [(keyPath: String, value: String, tag: UInt8)] = [
                ("credentialSubject.name", name, 2),
                ("credentialSubject.species", species, 2),
                ("credentialSubject.breedVbo", breedVbo, 2),
                ("credentialSubject.breedLabel", breedLabel, 2),
                ("credentialSubject.sex", sex, 2),
                ("credentialSubject.neuterStatus", neuterStatus, 2),
                ("credentialSubject.dateOfBirth", dateOfBirth, 2),
            ]
            for (index, weight) in weightHistory.enumerated() {
                values.append(("credentialSubject.weightHistory[\(index)].unit", weight.unit, 2))
                values.append(("credentialSubject.weightHistory[\(index)].value", weight.value, 4))
                values.append(("credentialSubject.weightHistory[\(index)].measuredOn", weight.measuredOn, 2))
            }
            values += [
                ("credentialSubject.microchip.code", microchip.code, 2),
                ("credentialSubject.microchip.standard", microchip.standard, 2),
                ("credentialSubject.microchip.implantDate", microchip.implantDate, 2),
                ("credentialSubject.microchip.bodyLocation", microchip.bodyLocation, 2),
            ]
            return values.filter { !$0.value.isEmpty }
        }
    }

    /// Non-consuming `GET /p/<token>` response. The parser accepts both the target nested `pet`
    /// shape and the stored-session `petName` + `profile` + `microchip` shape during rollout.
    struct DogTagIssueSession {
        let sessionId: String
        let dogTagId: String
        let status: String
        let pet: IssuePet
        let ownerIdentity: OwnerIdentity
        /// D1: the salted identity leaves to fold into `R`. Empty when the vet collected none (the
        /// bind then degrades to the pet-only contract).
        let identityLeaves: [IdentityLeaf]
    }

    /// Final owner-hidden issuance result after the session reports `bound` with a transaction hash.
    struct DogTagIssue {
        let dogTagId: String
        let root: String
        let txHash: String
        let status: String
        let bound: Bool
    }

    enum CustodialBindResult {
        case accepted(DogTagIssue)
        case inconclusive
        case rejected(statusCode: Int, body: String)
    }

    static func resolveDogTagIssue(host: String, token: String) async -> DogTagIssueSession? {
        guard !token.isEmpty else { return nil }
        let resp = await Http.getJSON("\(host)/p/\(token)")
        guard resp.ok, let data = resp.body.data(using: .utf8),
              let object = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any] else {
            return nil
        }
        let nestedPet = object["pet"] as? [String: Any]
        let topProfile = object["profile"] as? [String: Any]
        guard nestedPet != nil || topProfile != nil,
              let identity = (object["ownerIdentity"] as? [String: Any])
                ?? (object["owner_identity"] as? [String: Any]) else {
            // An older backend returned only ids/status. Issuing from that shape would silently
            // commit an owner-control-only tree and permanently discard the vet's pet metadata.
            return nil
        }
        let profile = (nestedPet?["profile"] as? [String: Any])
            ?? topProfile
            ?? nestedPet
            ?? [:]
        let microchip = (nestedPet?["microchip"] as? [String: Any])
            ?? (profile["microchip"] as? [String: Any])
            ?? (object["microchip"] as? [String: Any])
            ?? [:]
        let rawWeights = (profile["weightHistory"] as? [[String: Any]])
            ?? (nestedPet?["weightHistory"] as? [[String: Any]])
            ?? (object["weightHistory"] as? [[String: Any]])
            ?? []
        let dogTagId = jsonString(object["dogTagId"] ?? object["dog_tag_id"])
        let sessionId = jsonString(object["sessionId"] ?? object["session_id"])
        guard !dogTagId.isEmpty, !sessionId.isEmpty else { return nil }
        let pet = IssuePet(
                name: jsonString(nestedPet?["name"] ?? profile["name"]
                    ?? object["petName"] ?? object["pet_name"]),
                species: jsonString(nestedPet?["species"] ?? profile["species"]),
                breedVbo: jsonString(nestedPet?["breedVbo"] ?? profile["breedVbo"]
                    ?? profile["breed_vbo"]),
                breedLabel: jsonString(
                    nestedPet?["breedLabel"] ?? profile["breedLabel"]
                        ?? profile["breed_label"] ?? profile["breed"]),
                sex: jsonString(nestedPet?["sex"] ?? profile["sex"]),
                neuterStatus: jsonString(nestedPet?["neuterStatus"] ?? profile["neuterStatus"]
                    ?? profile["neuter_status"]),
                dateOfBirth: jsonString(nestedPet?["dateOfBirth"] ?? profile["dateOfBirth"]
                    ?? profile["date_of_birth"]),
                weightHistory: rawWeights.map { weight in
                    IssueWeight(
                        unit: jsonString(weight["unit"]),
                        value: jsonString(weight["value"]),
                        measuredOn: jsonString(weight["measuredOn"] ?? weight["measured_on"]))
                },
                microchip: IssueMicrochip(
                    code: jsonString(microchip["code"]),
                    standard: jsonString(microchip["standard"]),
                    implantDate: jsonString(microchip["implantDate"] ?? microchip["implant_date"]),
                    bodyLocation: jsonString(microchip["bodyLocation"] ?? microchip["body_location"])))
        guard !pet.name.isEmpty, !pet.profileAttributeValues.isEmpty else { return nil }
        // D1: the vet-salted identity leaves the device must fold into R. Each entry needs its full
        // {keyPath, saltHex, value} triple - a leaf missing any of them cannot be folded OR proven,
        // so it is dropped only if empty-keyed; a malformed salt surfaces later as an FFI error.
        let identityLeaves: [IdentityLeaf] = ((object["identityLeaves"] as? [[String: Any]]) ?? [])
            .compactMap { leaf in
                let keyPath = jsonString(leaf["keyPath"] ?? leaf["key_path"])
                let saltHex = jsonString(leaf["saltHex"] ?? leaf["salt_hex"])
                guard !keyPath.isEmpty, !saltHex.isEmpty else { return nil }
                let tag = (leaf["tag"] as? NSNumber)?.uint8Value ?? 2
                return IdentityLeaf(
                    keyPath: keyPath,
                    saltHex: saltHex,
                    tag: tag,
                    value: jsonString(leaf["value"]))
            }
        return DogTagIssueSession(
            sessionId: sessionId,
            dogTagId: dogTagId,
            status: jsonString(object["status"]),
            pet: pet,
            ownerIdentity: OwnerIdentity(
                countryOfIdentification: jsonString(
                    identity["countryOfIdentification"] ?? identity["country_of_identification"]),
                identification: jsonString(identity["identification"]),
                name: jsonString(identity["name"])),
            identityLeaves: identityLeaves)
    }

    /// POST <host>/profiles/issue/custodial-bind {token, root, leaves, reservedLeafHashes}. The
    /// device wallet and signature never cross this boundary; ownership is the reserved secret
    /// triple committed inside `root`. `leaves` opens EVERY attribute leaf of the tree (pet and
    /// identity alike) and `reservedLeafHashes` names the owner-control triple's leaf hashes
    /// OPAQUELY - the vet rebuilds `R` from them (the D1 full-leaf-list attestation-integrity
    /// gate); the reserved leaves' preimages never cross this boundary.
    static func bindDogTagIssue(
        host: String,
        token: String,
        root: String,
        leaves: [[String: Any]],
        reservedLeafHashes: [String]
    ) async -> CustodialBindResult {
        guard !token.isEmpty, !root.isEmpty else {
            return .rejected(statusCode: -1, body: "missing token or root")
        }
        let body: [String: Any] = [
            "token": token,
            "root": root,
            "leaves": leaves,
            "reservedLeafHashes": reservedLeafHashes,
        ]
        guard let raw = try? JSONSerialization.data(withJSONObject: body),
              let bodyStr = String(data: raw, encoding: .utf8) else {
            return .rejected(statusCode: -1, body: "could not encode request")
        }
        let resp = await Http.postJSON("\(host)/profiles/issue/custodial-bind", body: bodyStr, timeout: 20)
        if resp.code < 0 || resp.code == 410 { return .inconclusive }
        guard resp.ok else { return .rejected(statusCode: resp.code, body: resp.body) }
        guard let d = resp.body.data(using: .utf8),
              let o = (try? JSONSerialization.jsonObject(with: d)) as? [String: Any] else {
            return .inconclusive
        }
        let dogTagId = jsonString(o["dogTagId"] ?? o["dog_tag_id"])
        let returnedRoot = jsonString(o["root"] ?? o["R"])
        guard !dogTagId.isEmpty, !returnedRoot.isEmpty else {
            return .inconclusive
        }
        return .accepted(DogTagIssue(
            dogTagId: dogTagId,
            root: returnedRoot,
            txHash: jsonString(o["txHash"] ?? o["tx_hash"]),
            status: jsonString(o["status"]),
            bound: (o["bound"] as? Bool) ?? false))
    }

    private static func jsonString(_ value: Any?) -> String {
        if let string = value as? String { return string }
        if let number = value as? NSNumber { return number.stringValue }
        return ""
    }

    /// The platform-OWNED, UNVERIFIED discovery claims from the resolve GET's `unverifiedClaims` block
    /// (M7 §5.2). NONE of these is authority: they are validated against the dogtag
    /// `ProtocolRegistry` anchor before the app acts. Missing claims fail closed. Deliberately NOT
    /// named `ConvenienceClaims` — that is the FFI record `validateDiscovery` consumes; this is the
    /// raw parse the caller maps into it.
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
        /// The platform-owned claims are always required by the owner-hidden flow and are validated
        /// against the on-chain ProtocolRegistry before proving.
        let claims: UnverifiedClaims?
    }

    /// GET <host>/x/<token> → export-session metadata (non-consuming). Nil on failure.
    static func resolveExportSession(host: String, token: String) async -> ExportSession? {
        guard !token.isEmpty else { return nil }
        let resp = await Http.getJSON("\(host)/x/\(token)")
        guard resp.ok, let d = resp.body.data(using: .utf8),
              let o = (try? JSONSerialization.jsonObject(with: d)) as? [String: Any] else { return nil }
        // Missing claims are rejected by the single owner-hidden flow.
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
            claims: claims)
    }

    /// Submit the owner-hidden proof directly to the verifier host from the scanned QR.
    /// `/v1/verify/consent` is the sole canonical route: it accepts the `{exportToken, proof}`
    /// payload and resolves the session from the owner's one-time export token (the migration-era
    /// `/v1/verify/consent/levelb` alias is gone and 404s).
    static func postVerifyConsentToHost(host: String, payloadJson: String) async -> Http.Response {
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

// MARK: - Provider directory

/// Native adapter for the shared ProviderDirectory contract.
///
/// The screen receives this seam and never performs HTTP itself. `read()` accepts no query, and the
/// endpoint builder rejects a configured base that already contains one, so neither a current nor a
/// chosen position has a request-shaped place to go. A failed live read may replay the same universal,
/// full-set snapshot while its hard TTL remains open; that replay is labelled `.stored`.
struct CentralProviderDirectory: ProviderDirectoryReading {
    let source: ProviderDirectorySource = .central
    static let defaultTtl: TimeInterval = 15 * 60

    private let baseURL: String
    private let ttl: TimeInterval
    private let now: () -> Date

    init(
        baseURL: String = AppConfig.centralApi,
        ttl: TimeInterval = Self.defaultTtl,
        now: @escaping () -> Date = Date.init
    ) {
        self.baseURL = baseURL
        self.ttl = ttl
        self.now = now
    }

    func read() async -> ProviderDirectoryResult {
        let attemptedAt = now()
        guard ttl.isFinite, ttl > 0,
              let endpoint = Self.endpoint(baseURL: baseURL) else {
            return fallbackOrUnavailable(
                reason: .sourceUnavailable,
                detail: "The provider directory is not configured with a valid query-free URL",
                attemptedAt: attemptedAt
            )
        }

        let response = await Http.getJSON(endpoint)
        guard response.ok else {
            return fallbackOrUnavailable(
                reason: .sourceUnavailable,
                detail: response.code < 0
                    ? "The provider directory could not be reached"
                    : "The provider directory returned HTTP \(response.code)",
                attemptedAt: attemptedAt
            )
        }
        guard let data = response.body.data(using: .utf8),
              let object = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any],
              let providers = Self.parseProviders(object) else {
            return fallbackOrUnavailable(
                reason: .malformedResponse,
                detail: "The provider directory returned an invalid response; it was not treated as empty",
                attemptedAt: attemptedAt
            )
        }

        let readAt = now()
        let snapshot = ProviderDirectorySnapshot(
            source: .central,
            providers: providers,
            observation: .live,
            blockNumber: nil,
            readAt: readAt,
            expiresAt: readAt.addingTimeInterval(ttl)
        )
        ProviderDirectoryMemoryCache.write(snapshot, namespace: cacheNamespace)
        return providers.isEmpty ? .empty(snapshot) : .found(snapshot)
    }

    /// The exact request target. It is always the full-set endpoint and never has a query string.
    static func endpoint(baseURL: String) -> String? {
        let raw = baseURL.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !raw.isEmpty, var components = URLComponents(string: raw),
              components.query == nil, components.fragment == nil,
              let scheme = components.scheme?.lowercased(),
              scheme == "https" || scheme == "http",
              components.host?.isEmpty == false else { return nil }
        components.path = components.path
            .trimmingCharacters(in: CharacterSet(charactersIn: "/"))
        components.path = "/" + [components.path, "v1/businesses"]
            .filter { !$0.isEmpty && $0 != "/" }
            .joined(separator: "/")
        components.query = nil
        components.fragment = nil
        return components.url?.absoluteString
    }

    private var cacheNamespace: String {
        "central:\(Self.endpoint(baseURL: baseURL) ?? "unresolved-base")"
    }

    private func fallbackOrUnavailable(
        reason: ProviderDirectoryUnavailableReason,
        detail: String,
        attemptedAt: Date
    ) -> ProviderDirectoryResult {
        if var cached = ProviderDirectoryMemoryCache.read(namespace: cacheNamespace, now: attemptedAt) {
            cached.observation = .stored
            return cached.providers.isEmpty ? .empty(cached) : .found(cached)
        }
        return .unavailable(ProviderDirectoryUnavailable(
            source: .central,
            reason: reason,
            detail: detail,
            attemptedAt: attemptedAt
        ))
    }

    /// All-or-nothing validation, except that a genuinely absent/null `geo` is the valid contact-only
    /// case. A malformed coordinate object is not silently dropped or changed into a location-less row.
    ///
    /// A repeated `businessId` is malformed for the same reason: the id is this screen's list
    /// identity, so keeping both rows would corrupt rendering instead of reporting a bad response.
    static func parseProviders(_ object: [String: Any]) -> [DirectoryProvider]? {
        guard let rows = object["businesses"] as? [Any] else { return nil }
        var providers: [DirectoryProvider] = []
        var seenIds = Set<String>()
        providers.reserveCapacity(rows.count)

        for raw in rows {
            guard let row = raw as? [String: Any],
                  let providerId = row["businessId"] as? String,
                  seenIds.insert(providerId).inserted,
                  let kind = row["type"] as? String,
                  let name = row["name"] as? String,
                  let services = row["services"] as? [String],
                  let geo = parseGeo(row["geo"]),
                  let domain = row["domain"] as? String,
                  let contact = parseContact(row["contact"]) else {
                return nil
            }

            let claimedDomain = domain.trimmingCharacters(in: .whitespacesAndNewlines)
            providers.append(DirectoryProvider(
                providerId: providerId,
                kind: kind,
                name: name,
                geo: geo,
                services: services,
                domain: claimedDomain.isEmpty ? nil : claimedDomain,
                // Today's central response carries no delisting fact. Ignore any unrelated extra
                // wire field rather than manufacturing a current-standing claim from this source.
                active: nil,
                contact: contact,
                // Central does not carry a resolved on-chain/DNS observation and reads no chain state
                // at all. A blank domain is a fact about THIS LISTING, never the on-chain
                // `noDomainClaimed`; a nonblank claim stays neutral/unavailable rather than being
                // promoted to verified merely because the directory echoed it.
                bindingState: claimedDomain.isEmpty ? .noDomainListed : .unavailable
            ))
        }
        return providers
    }

    /// Optional-success parser: the outer optional means valid/invalid, the inner value is absence.
    private static func optionalString(_ raw: Any?) -> String?? {
        if raw == nil || raw is NSNull { return .some(nil) }
        guard let value = raw as? String else { return nil }
        let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
        return .some(trimmed.isEmpty ? nil : trimmed)
    }

    /// Optional-success parser matching `optionalString`: `.some(nil)` is a real contact-only row.
    private static func parseGeo(_ raw: Any?) -> NearbyPoint?? {
        if raw == nil || raw is NSNull { return .some(nil) }
        guard let object = raw as? [String: Any] else { return nil }
        guard let lat = number(object["lat"]), let lng = number(object["lng"]) else { return nil }
        let point = NearbyPoint(lat: lat, lng: lng)
        return point.isValid ? .some(point) : nil
    }

    private static func number(_ raw: Any?) -> Double? {
        guard !(raw is Bool), let number = raw as? NSNumber else { return nil }
        let value = number.doubleValue
        return value.isFinite ? value : nil
    }

    private static func parseContact(_ raw: Any?) -> ProviderContact? {
        if raw == nil || raw is NSNull { return ProviderContact() }
        guard let object = raw as? [String: Any],
              let phone = optionalString(object["phone"]),
              let whatsapp = optionalString(object["whatsapp"]),
              let telegram = optionalString(object["telegram"]),
              let email = optionalString(object["email"]),
              let website = optionalString(object["website"]) else {
            return nil
        }
        return ProviderContact(
            phone: phone,
            whatsapp: whatsapp,
            telegram: telegram,
            email: email,
            website: website
        )
    }
}

/// One process-local full-set snapshot per configured directory. The value is never keyed by a
/// position, name search, radius, geohash, or viewport.
private enum ProviderDirectoryMemoryCache {
    private static let lock = NSLock()
    private static var entries: [String: ProviderDirectorySnapshot] = [:]

    static func write(_ snapshot: ProviderDirectorySnapshot, namespace: String) {
        lock.lock()
        entries[namespace] = snapshot
        lock.unlock()
    }

    static func read(namespace: String, now: Date) -> ProviderDirectorySnapshot? {
        lock.lock()
        defer { lock.unlock() }
        guard let snapshot = entries[namespace],
              let expiresAt = snapshot.expiresAt,
              snapshot.readAt <= now,
              now < expiresAt else {
            entries.removeValue(forKey: namespace)
            return nil
        }
        return snapshot
    }
}
