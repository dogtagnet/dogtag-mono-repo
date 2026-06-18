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
}
