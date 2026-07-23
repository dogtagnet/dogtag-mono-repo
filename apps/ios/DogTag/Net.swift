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
    private static let consumedSelector = "0x4648c943"         // keccak256("consumed(bytes32)")[:4]
    private static let profileRootSelector = "0x85105cb3"      // keccak256("profileRoot(uint256)")[:4]

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
    /// Human identity collected by the issuing vet, shown to the owner before binding. The values
    /// the device actually FOLDS into `R` come from `identityLeaves` (below), which carry the
    /// vet-generated salts.
    struct OwnerIdentity {
        let countryOfIdentification: String
        let identification: String
        let name: String
    }

    /// D1: one vet-salted identity attribute leaf the device MUST fold into `R` alongside the pet
    /// attributes. `{keyPath, saltHex, tag, value}` - the salt is the VET's (it re-verifies each
    /// leaf's inclusion in `R` at bind and refuses the mint otherwise), and the device persists the
    /// triple as a disclosure opening.
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

    /// POST <host>/profiles/issue/custodial-bind {token, root, identityProofs}. The device wallet
    /// and signature never cross this boundary; ownership is the reserved secret triple committed
    /// inside `root`. `identityProofs` carries ONLY Merkle paths (`[{keyPath, proof}]`) - the vet
    /// recomputes each identity leaf from its own stored `{keyPath, salt, value}`, so no identity
    /// value or salt is re-sent here.
    static func bindDogTagIssue(
        host: String,
        token: String,
        root: String,
        identityProofs: [[String: Any]] = []
    ) async -> CustodialBindResult {
        guard !token.isEmpty, !root.isEmpty else {
            return .rejected(statusCode: -1, body: "missing token or root")
        }
        var body: [String: Any] = ["token": token, "root": root]
        if !identityProofs.isEmpty {
            body["identityProofs"] = identityProofs
        }
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

    /// Submit the owner-hidden proof directly to the verifier host from the scanned QR. The Level-B
    /// route is the one that accepts an `{exportToken, proof}` payload under the owner's one-time
    /// export token; the non-`levelb` `/v1/verify/consent` requires `consent`+`sig` and refuses a
    /// `levelb` session.
    static func postVerifyConsentToHost(host: String, payloadJson: String) async -> Http.Response {
        await Http.postJSON("\(host)/v1/verify/consent/levelb", body: payloadJson, timeout: 20)
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
