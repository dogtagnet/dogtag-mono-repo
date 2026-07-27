import Foundation

/// Implements the scan-to-import flow (impl §6.5). Fetch the wrapped doc with the Bearer JWT and run
/// the verification pillars: INTEGRITY (offline Rust FFI `verifyIntegrity`), ISSUANCE (on-chain
/// `DogTagIssuer.isValid` over ROAX RPC) and ISSUER WHITELIST (on-chain `issuedBy` → the app's own
/// `IssuerRegistry`). Store the record under the matching pet, grouped by recordType.
///
/// Every pillar is tri-state and none may be skipped: a pillar that does not resolve yields an
/// indeterminate verdict, never a pass.
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
        // `var` only so the issuer-whitelist pillar below can tighten it; the mapping itself is
        // deliberately left exactly as-is.
        var verdict: String
        switch onchain {
        case _ where !integrityOk: verdict = "INVALID"
        case .invalid: verdict = "INVALID"
        case .valid: verdict = "VALID"
        case .unknown: verdict = "VALID"   // integrity passed; chain unreachable -> accept with caveat
        }

        // 4. ISSUER-WHITELIST pillar (on-chain, MANDATORY).
        //
        // Integrity and issuance together still accept a forged authority: the `issuer` block is
        // outside the Merkle root, so relabelling the issuer - or pointing `documentStore` at a
        // contract that returns true from `isValid` - passes both. This pillar asks the chain who
        // actually issued the root and whether that signer is whitelisted for this record type in the
        // app's own bundled registry.
        let whitelist = await RoaxRpc.issuerWhitelistPillar(
            rpcUrl: rpcUrl, issuerRegistry: RoaxConfig.load().issuerRegistry,
            documentStore: doc.documentStore, root: doc.merkleRoot, recordType: doc.recordType)
        verdict = foldIssuerWhitelist(verdict, whitelist)

        let chainNote: String
        switch onchain {
        case .valid: chainNote = "on-chain isValid: yes"
        case .invalid: chainNote = "on-chain isValid: NO (revoked/not anchored)"
        case let .unknown(r): chainNote = "on-chain isValid: unknown (\(r))"
        }
        let whitelistNote: String
        switch whitelist {
        case .valid: whitelistNote = "issuer whitelist: yes"
        case .invalid: whitelistNote = "issuer whitelist: NO (signer not authorized for this record type)"
        case let .unknown(r): whitelistNote = "issuer whitelist: unresolved (\(r))"
        }
        let issuerLabel = doc.issuerDomain.isEmpty ? doc.issuerName : doc.issuerDomain
        let detail = "integrity: \(integrity) · \(chainNote) · \(whitelistNote) · issuer \(issuerLabel)"

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

    /// Fold the issuer-whitelist pillar into the verdict the integrity + issuance pillars produced.
    ///
    /// Deliberately a separate, MONOTONE step rather than another arm of the issuance mapping: it can
    /// only make a verdict stricter, never looser, so it composes with whatever that mapping decides -
    /// including a stricter future mapping (e.g. once an unresolved chain read stops yielding VALID).
    ///
    /// - `.valid`   the issuing signer is authorized: the verdict stands as-is.
    /// - `.invalid` resolved, and that signer may NOT issue this record type: a real authenticity
    ///              failure, so INVALID.
    /// - `.unknown` the pillar did not resolve. An unanswered check is never a passed check, so a
    ///              would-be VALID degrades to UNVERIFIED; anything already worse stands.
    static func foldIssuerWhitelist(_ verdict: String, _ pillar: RoaxRpc.Result) -> String {
        switch pillar {
        case .valid: return verdict
        case .invalid: return "INVALID"
        case .unknown: return verdict == "VALID" ? "UNVERIFIED" : verdict
        }
    }
}
