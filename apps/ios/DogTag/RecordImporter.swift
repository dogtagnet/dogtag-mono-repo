import Foundation

/// Implements the scan-to-import flow (impl §6.5). Fetch the wrapped doc with the Bearer JWT and run
/// the verification pillars: INTEGRITY (offline Rust FFI `verifyIntegrity`) + ISSUANCE (on-chain
/// `DogTagIssuer.isValid` over ROAX RPC). Store the record under the matching pet, grouped by recordType.
enum RecordImporter {
    struct ImportResult {
        let ok: Bool
        let verdict: String          // "VALID" / "INVALID" / "UNVERIFIED"
        let detail: String
        let credential: Credential?
    }

    static func `import`(_ req: QrPayload, rpcUrl: String = AppConfig.roaxRpc) async -> ImportResult {
        // Resolve the fetch URL + a fallback local id from the QR shape:
        //   - SHORT token: GET <host>/r/<token> (no Bearer) — server consumes the one-time token.
        //   - legacy JWT:  GET <host>/records/{recordId} with the Bearer record-JWT (back-compat).
        let url: String
        let bearer: String?
        let fallbackId: String
        switch req {
        case let .importRecordToken(host, token):
            url = "\(host)/r/\(token)"; bearer = nil; fallbackId = token
        case let .importRecord(host, recordId, jwt):
            url = "\(host)/records/\(recordId)"; bearer = jwt; fallbackId = recordId
        default:
            return ImportResult(ok: false, verdict: "UNVERIFIED", detail: "not an import QR", credential: nil)
        }

        let resp = await Http.getJSON(url, bearer: bearer)
        guard resp.ok else {
            return ImportResult(ok: false, verdict: "UNVERIFIED",
                                detail: "GET \(url) -> \(resp.code): \(resp.body.prefix(120))", credential: nil)
        }
        let wrappedJson = resp.body
        guard let doc = WrappedDoc(json: wrappedJson) else {
            return ImportResult(ok: false, verdict: "UNVERIFIED", detail: "bad wrapped doc", credential: nil)
        }

        // 2. INTEGRITY pillar (offline, Rust FFI).
        let integrity = (try? verifyIntegrity(wrappedDocJson: wrappedJson)) ?? "INVALID"

        // 3. ISSUANCE pillar (on-chain isValid via ROAX RPC).
        let onchain = await RoaxRpc.isValid(rpcUrl: rpcUrl, documentStore: doc.documentStore, root: doc.merkleRoot)

        let integrityOk = integrity == "VALID"
        let verdict: String
        switch onchain {
        case _ where !integrityOk: verdict = "INVALID"
        case .invalid: verdict = "INVALID"
        case .valid: verdict = "VALID"
        case .unknown: verdict = "VALID"   // integrity passed; chain unreachable -> accept with caveat
        }

        let chainNote: String
        switch onchain {
        case .valid: chainNote = "on-chain isValid: yes"
        case .invalid: chainNote = "on-chain isValid: NO (revoked/not anchored)"
        case let .unknown(r): chainNote = "on-chain isValid: unknown (\(r))"
        }
        let issuerLabel = doc.issuerDomain.isEmpty ? doc.issuerName : doc.issuerDomain
        let detail = "integrity: \(integrity) · \(chainNote) · issuer \(issuerLabel)"

        let group = CredentialGroup.from(recordType: doc.recordType)
        let dogTagId = doc.dogTagId.isEmpty ? fallbackId : doc.dogTagId
        let cred = Credential(
            id: fallbackId,
            dogTagId: dogTagId,
            group: group,
            recordType: doc.recordType.isEmpty ? "RECORD" : doc.recordType,
            title: doc.displayTitle(),
            subtitle: doc.recordType.isEmpty ? "Imported record" : doc.recordType,
            issuer: doc.issuerName,
            issuedOn: "",
            credentialRoot: doc.merkleRoot,
            verdict: verdict,
            wrappedDocJson: wrappedJson
        )
        return ImportResult(ok: verdict != "INVALID", verdict: verdict, detail: detail, credential: cred)
    }

    /// The outcome of polling the chain for the async SBT mint.
    enum MintPoll {
        case confirmed   // profileRoot(dogTagId) == root AND ownerOf(dogTagId) == wallet
        case timeout     // polled the full window without observing both anchors
    }

    /// Poll the DogTagSBT for the ASYNC mint after a bind. The vet responds immediately (status
    /// "minting") and mints in the background (~12-24s on ROAX), so the anchors appear only after a few
    /// blocks. Poll `profileRoot(dogTagId)` + `ownerOf(dogTagId)` every `intervalMs` for up to
    /// `timeoutMs`, succeeding once BOTH match (case-insensitive). A miss is retried — NOT a hard fail.
    static func pollSbtMint(
        dogTagId: String, expectedRoot: String, walletAddress: String, dogTagSbt: String,
        rpcUrl: String = AppConfig.roaxRpc, intervalMs: UInt64 = 3000, timeoutMs: UInt64 = 120_000
    ) async -> MintPoll {
        // The SBT is minted under the canonical on-chain id `field_of_value(dogTagId)` (matching the
        // issuance/export path), so the anchor reads must use the field-hashed id, not the raw handle.
        // `dogTagIdFieldHex` returns a 0x..32-byte hex word, which RoaxRpc.padUint encodes directly.
        let onchainDogTagId = (try? dogTagIdFieldHex(dogTagIdDec: dogTagId)) ?? dogTagId
        let deadline = Date().addingTimeInterval(Double(timeoutMs) / 1000.0)
        while Date() < deadline {
            let onchainRoot = await RoaxRpc.profileRoot(rpcUrl: rpcUrl, dogTagSbt: dogTagSbt, dogTagId: onchainDogTagId)
            let onchainOwner = await RoaxRpc.ownerOf(rpcUrl: rpcUrl, dogTagSbt: dogTagSbt, dogTagId: onchainDogTagId)
            let rootOk = onchainRoot != nil && onchainRoot!.lowercased() == expectedRoot.lowercased()
            let ownerOk = onchainOwner != nil && onchainOwner!.lowercased() == walletAddress.lowercased()
            if rootOk && ownerOk { return .confirmed }
            try? await Task.sleep(nanoseconds: intervalMs * 1_000_000)
        }
        return .timeout
    }

    /// The vet-issues-the-dog-tag (DOG_PROFILE) verify+store branch. Unlike the import path above, a
    /// DOG_PROFILE is anchored in the DogTagSBT — NOT in a DogTagIssuer clone — so the issuance pillar is:
    ///   1. INTEGRITY (offline, Rust FFI) == "VALID", and the doc's `signature.merkleRoot` must equal
    ///      the bind-returned `expectedRoot`.
    ///   2. SBT ANCHOR (on-chain): `DogTagSBT.profileRoot(dogTagId)` == `expectedRoot` AND
    ///      `DogTagSBT.ownerOf(dogTagId)` == `walletAddress`.
    static func verifyIssuedDogTag(
        wrappedDocJson: String, dogTagId: String, expectedRoot: String, walletAddress: String,
        dogTagSbt: String, rpcUrl: String = AppConfig.roaxRpc
    ) async -> ImportResult {
        guard let doc = WrappedDoc(json: wrappedDocJson) else {
            return ImportResult(ok: false, verdict: "UNVERIFIED", detail: "bad wrapped doc", credential: nil)
        }

        // 1. INTEGRITY (offline) + root consistency (doc.merkleRoot == bind root).
        let integrity = (try? verifyIntegrity(wrappedDocJson: wrappedDocJson)) ?? "INVALID"
        let integrityOk = integrity == "VALID"
        let rootMatchesDoc = !expectedRoot.isEmpty && doc.merkleRoot.lowercased() == expectedRoot.lowercased()

        // 2. SBT ANCHOR (on-chain) — profileRoot(id) == root AND ownerOf(id) == wallet, where the id is
        // the canonical `field_of_value(dogTagId)` the SBT was minted under (0x..hex, which RoaxRpc.padUint
        // encodes directly). The raw `dogTagId` is still used for the credential/record fields.
        let onchainDogTagId = (try? dogTagIdFieldHex(dogTagIdDec: dogTagId)) ?? dogTagId
        let onchainRoot = await RoaxRpc.profileRoot(rpcUrl: rpcUrl, dogTagSbt: dogTagSbt, dogTagId: onchainDogTagId)
        let onchainOwner = await RoaxRpc.ownerOf(rpcUrl: rpcUrl, dogTagSbt: dogTagSbt, dogTagId: onchainDogTagId)
        let rpcReached = onchainRoot != nil && onchainOwner != nil
        let rootOk = onchainRoot != nil && onchainRoot!.lowercased() == expectedRoot.lowercased()
        let ownerOk = onchainOwner != nil && onchainOwner!.lowercased() == walletAddress.lowercased()

        let verdict: String
        if !integrityOk || !rootMatchesDoc { verdict = "INVALID" }
        else if rpcReached && (!rootOk || !ownerOk) { verdict = "INVALID" }
        else { verdict = "VALID" }   // chain unreachable -> accept with caveat

        let chainNote: String
        if !rpcReached { chainNote = "SBT verify: unknown (RPC unreachable)" }
        else if rootOk && ownerOk { chainNote = "SBT verify: profileRoot + ownerOf match" }
        else if !rootOk { chainNote = "SBT verify: profileRoot MISMATCH" }
        else { chainNote = "SBT verify: ownerOf MISMATCH (\(onchainOwner ?? ""))" }
        let issuerLabel = doc.issuerDomain.isEmpty ? doc.issuerName : doc.issuerDomain
        let detail = "integrity: \(integrity) · \(chainNote) · issuer \(issuerLabel)"

        let cred = Credential(
            id: dogTagId.isEmpty ? expectedRoot : dogTagId,
            dogTagId: dogTagId,
            group: CredentialGroup.from(recordType: doc.recordType),
            recordType: doc.recordType.isEmpty ? "DOG_PROFILE" : doc.recordType,
            title: doc.displayTitle(),
            subtitle: doc.recordType.isEmpty ? "Dog tag" : doc.recordType,
            issuer: doc.issuerName,
            issuedOn: "",
            credentialRoot: doc.merkleRoot.isEmpty ? expectedRoot : doc.merkleRoot,
            verdict: verdict,
            wrappedDocJson: wrappedDocJson)
        return ImportResult(ok: verdict != "INVALID", verdict: verdict, detail: detail, credential: cred)
    }
}
