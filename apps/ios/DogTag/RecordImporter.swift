import Foundation

/// Implements the scan-to-import flow (impl §6.5). Fetch the wrapped doc with the Bearer JWT and run
/// the verification pillars: INTEGRITY (offline Rust FFI `verifyIntegrity`), ISSUANCE (on-chain
/// `DogTagIssuer.isValid` over ROAX RPC) and ISSUER WHITELIST (the app's own
/// `DogTagIssuerFactory.rootIssuer` → that clone's `recordType()`/`issuedBy` → the app's own
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

    static func `import`(_ req: QrPayload, rpcUrl: String = RpcEndpointSettings.rpcUrl()) async -> ImportResult {
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

        // NO FAIL-OPEN: an unreachable RPC means we did not check, which is NOT the same as checking
        // and finding the record good. It records UNVERIFIED with the reason, never VALID.
        let integrityOk = integrity == "VALID"
        // `var` only so the issuer-whitelist pillar below can tighten the pair; the mapping arms
        // themselves are left exactly as the refresh-from-chain work wrote them.
        var verdict: String
        var reason: String
        if !integrityOk {
            verdict = "INVALID"
            reason = "the record's contents do not match its signed Merkle root"
        } else {
            switch onchain {
            case .valid: verdict = "VALID"; reason = "anchored on ROAX and not revoked"
            case .invalid: verdict = "INVALID"; reason = "revoked or not anchored on ROAX"
            case let .unknown(r): verdict = "UNVERIFIED"; reason = "could not reach the chain (\(r))"
            }
        }

        // 4. ISSUER-WHITELIST pillar (on-chain, MANDATORY).
        //
        // Integrity and issuance together still accept a forged authority: the `issuer` block is
        // outside the Merkle root, so relabelling the issuer - or pointing `documentStore` at a
        // contract that returns true from `isValid` - passes both. This pillar resolves the issuing
        // clone from the app's OWN factory, then asks THAT clone what record type it holds and who
        // issued the root, and checks that signer against the app's own bundled registry.
        let roax = RoaxConfig.load()
        let whitelist = await RoaxRpc.issuerWhitelistPillar(
            rpcUrl: rpcUrl, issuerRegistry: roax.issuerRegistry,
            issuerFactory: roax.issuerFactory,
            documentStore: doc.documentStore, root: doc.merkleRoot, recordType: doc.recordType)
        // Same fold the refresh path uses - see `IssuerWhitelist`. Moves the reason with the verdict.
        (verdict, reason) = IssuerWhitelist.fold(verdict, reason, whitelist)

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
            wrappedDocJson: wrappedJson,
            importedAt: Stamp.now(),
            lastCheckedAt: Stamp.now(),
            verdictReason: reason
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
        IssuerWhitelist.fold(verdict, "", pillar).verdict
    }
}

/// THE issuer-whitelist decision. Both surfaces that produce a verdict - import and refresh - go
/// through here, deliberately.
///
/// They did not always. `CredentialRefresher` derived its own verdict from `isValid` alone, with no
/// whitelist pillar, so the first refresh UPGRADED a credential that import had correctly recorded as
/// UNVERIFIED back to VALID. The feature added so a revocation could reach the user was, on that
/// path, laundering an unverified credential into a trusted one - worse than the original fail-open,
/// which at least never overwrote a correct negative with a positive. Two parallel implementations of
/// one verdict rule is how that happened, so there is now exactly one.
enum IssuerWhitelist {
    /// Fold the pillar into a (verdict, reason) pair. MONOTONE: strictly downward, never upward, so
    /// it can only tighten whatever the integrity + issuance pillars already decided. The reason moves
    /// with the verdict - a degraded verdict carrying the old "anchored on ROAX and not revoked" text
    /// would be an over-claiming explanation stapled to a correct verdict.
    static func fold(
        _ verdict: String, _ reason: String, _ pillar: RoaxRpc.Result
    ) -> (verdict: String, reason: String) {
        switch pillar {
        case .valid:
            return (verdict, reason)
        case .invalid:
            return ("INVALID", "the address that issued this record is not authorised to issue this record type")
        case let .unknown(r):
            guard verdict == "VALID" else { return (verdict, reason) }
            return ("UNVERIFIED", "could not establish who issued this record (\(r))")
        }
    }

    /// Resolve the pillar on-chain and fold it, in one call. `pillar` is injectable so a test can
    /// drive the rule without a live RPC.
    static func evaluate(
        verdict: String, reason: String,
        rpcUrl: String, roax: RoaxConfig,
        documentStore: String, root: String, recordType: String,
        pillar: RoaxRpc.Result? = nil
    ) async -> (verdict: String, reason: String) {
        // Spelled out rather than `??`: that operator takes an autoclosure, which cannot be `async`.
        let resolved: RoaxRpc.Result
        if let pillar = pillar {
            resolved = pillar
        } else {
            resolved = await RoaxRpc.issuerWhitelistPillar(
                rpcUrl: rpcUrl, issuerRegistry: roax.issuerRegistry,
                issuerFactory: roax.issuerFactory,
                documentStore: documentStore, root: root, recordType: recordType)
        }
        return fold(verdict, reason, resolved)
    }
}

/// Re-reads the on-chain status of an ALREADY-STORED credential and returns an updated copy.
///
/// Without this the verdict is frozen at import: a record REVOKED afterwards keeps showing whatever
/// it showed on the day it was scanned, forever. A refresh recomputes both pillars from what the
/// phone already holds - INTEGRITY offline over the stored wrapped doc, and ISSUANCE as a live read
/// on ROAX - and stamps `lastCheckedAt` so the answer's age is visible.
///
/// It NEVER fails open. If the chain cannot be reached the result is UNVERIFIED with the reason:
/// "I could not check" is not "I checked and it is fine", and on a revocation check that distinction
/// is the whole point.
enum CredentialRefresher {
    static func refreshed(
        _ cred: Credential, roax: RoaxConfig, rpcUrl: String = RpcEndpointSettings.rpcUrl()
    ) async -> Credential {
        keepingEstablishedNegative(previous: cred, fresh: await derived(cred, roax: roax, rpcUrl: rpcUrl))
    }

    /// A refresh may LOWER a verdict freely - that is the whole point, and an unreachable chain must
    /// stop a stale VALID being claimed. But a NON-ANSWER must never make a credential look BETTER.
    ///
    /// Order the verdicts by severity: INVALID < UNVERIFIED < VALID. Re-deriving from scratch meant
    /// airplane mode plus one refresh tap replaced an established INVALID (revoked, or an unauthorised
    /// issuer) with UNVERIFIED and overwrote its reason - laundering a known-bad credential into
    /// "could not check". That is this branch's own defect class pointed backwards: a non-answer
    /// collapsing into a neighbouring state.
    ///
    /// Only non-answers are constrained. A DEFINITE outcome still sets the verdict freely, including
    /// INVALID -> VALID: a not-yet-anchored root can later anchor and a signer can later be
    /// whitelisted, so the guard must not freeze INVALID forever. `lastCheckedAt` is taken from the
    /// fresh result either way, so a kept verdict still shows that the check ran and how old it is.
    ///
    /// Applied ONCE to the final result rather than at each return site, so a future early return
    /// cannot bypass it.
    static func keepingEstablishedNegative(previous: Credential, fresh: Credential) -> Credential {
        guard fresh.verdict == "UNVERIFIED", previous.verdict == "INVALID" else { return fresh }
        var kept = fresh
        kept.verdict = previous.verdict
        kept.verdictReason = previous.verdictReason
        return kept
    }

    private static func derived(
        _ cred: Credential, roax: RoaxConfig, rpcUrl: String = RpcEndpointSettings.rpcUrl()
    ) async -> Credential {
        var out = cred
        out.lastCheckedAt = Stamp.now()

        guard let doc = WrappedDoc(json: cred.wrappedDocJson) else {
            return out.marked("UNVERIFIED", "the stored document could not be read")
        }

        // 1. INTEGRITY (offline) - the stored doc must still hash to its own signed root.
        let integrity = (try? verifyIntegrity(wrappedDocJson: cred.wrappedDocJson)) ?? "INVALID"
        guard integrity == "VALID" else {
            return out.marked("INVALID", "the record's contents do not match its signed Merkle root")
        }

        let root = doc.merkleRoot.isEmpty ? cred.credentialRoot : doc.merkleRoot
        guard !root.isEmpty else {
            return out.marked("UNVERIFIED", "this record carries no Merkle root to look up")
        }

        // 2. ISSUANCE (on-chain). Two anchor shapes: an ordinary record is anchored in a DogTagIssuer
        // clone (isValid(root)); a DOG_PROFILE is anchored in the DogTagSBT itself (profileRoot).
        if !doc.documentStore.isEmpty {
            let (v, r): (String, String)
            switch await RoaxRpc.isValid(rpcUrl: rpcUrl, documentStore: doc.documentStore, root: root) {
            case .valid: (v, r) = ("VALID", "anchored on ROAX and not revoked")
            case .invalid: (v, r) = ("INVALID", "revoked or no longer anchored on ROAX")
            case let .unknown(reason): (v, r) = ("UNVERIFIED", "could not reach the chain (\(reason))")
            }
            // 3. ISSUER WHITELIST - the SAME mandatory pillar import runs, through the SAME fold.
            //
            // Without it this path answered VALID from `isValid` alone, which meant the first refresh
            // silently UPGRADED a credential import had correctly marked UNVERIFIED for an unresolved
            // issuer. A refresh may lower a verdict, and may raise one only as far as the pillars
            // actually establish - never past them.
            //
            // Scoped to the clone path deliberately: the DOG_PROFILE branch below anchors in the SBT,
            // where `rootIssuer`/`issuedBy` do not exist, so applying the pillar there would resolve
            // indeterminate and turn every profile refresh into UNVERIFIED - the same over-claiming
            // mistake inverted.
            let (fv, fr) = await IssuerWhitelist.evaluate(
                verdict: v, reason: r, rpcUrl: rpcUrl, roax: roax,
                documentStore: doc.documentStore, root: root, recordType: doc.recordType)
            return out.marked(fv, fr)
        }

        guard !roax.dogTagSbt.isEmpty, !cred.dogTagId.isEmpty else {
            return out.marked("UNVERIFIED", "this record carries no on-chain anchor to re-check")
        }
        // The SBT is keyed by the canonical `field_of_value(dogTagId)`, not the raw handle. Without
        // that key there is no lookup to make, so the record is UNVERIFIED - never looked up under
        // the raw handle, which reads an unset slot and would read as a mismatch.
        guard let onchainId = try? dogTagIdFieldHex(dogTagIdDec: cred.dogTagId) else {
            return out.marked("UNVERIFIED", "this dog tag id could not be resolved on-chain")
        }
        guard let onchainRoot = await RoaxRpc.profileRoot(
            rpcUrl: rpcUrl, dogTagSbt: roax.dogTagSbt, dogTagId: onchainId) else {
            return out.marked("UNVERIFIED", "could not reach the chain (DogTagSBT profileRoot read failed)")
        }
        // An all-zero word is the UNSET slot: the tag is not anchored (yet), which is not the same
        // answer as an anchored root that disagrees with ours. Only the latter is INVALID.
        guard onchainRoot.dropFirst(2).contains(where: { $0 != "0" }) else {
            return out.marked("UNVERIFIED", "no profile root is anchored on-chain for this dog tag")
        }
        guard onchainRoot.lowercased() == root.lowercased() else {
            return out.marked("INVALID", "the dog tag's on-chain profile root no longer matches this record")
        }
        // The tag is custodial (owner-hidden), so there is no on-chain owner to compare against - the
        // profile-root match IS the anchor check, and the reason says exactly which check ran.
        return out.marked("VALID", "the dog tag's on-chain profile root matches this record")
    }
}

private extension Credential {
    /// Write a verdict and its reason together, so the pair can never drift apart.
    func marked(_ verdict: String, _ reason: String) -> Credential {
        var out = self
        out.verdict = verdict
        out.verdictReason = reason
        return out
    }
}
