package io.liberalize.dogtag.data

import io.liberalize.dogtag.net.Http
import io.liberalize.dogtag.net.RoaxRpc
import io.liberalize.dogtag.qr.QrPayload
import uniffi.dogtag_standard.verifyIntegrity

/**
 * Implements the scan-to-import flow (impl §6.5). Given a scanned vet/groomer record link, fetch the
 * wrapped doc with the Bearer JWT and run the verification pillars:
 *
 *   1. INTEGRITY (offline): recompute the Poseidon leaves + Merkle root via the Rust FFI
 *      `verifyIntegrity(wrappedDocJson)` -> "VALID" / "INVALID" (== signature.targetHash/merkleRoot).
 *   2. ISSUANCE (on-chain): `DogTagIssuer.isValid(merkleRoot)` over `issuer.documentStore` via ROAX RPC.
 *   3. ISSUER WHITELIST (on-chain): resolve the issuing clone from the app's OWN
 *      `DogTagIssuerFactory.rootIssuer(merkleRoot)`, then `recordType()` + `issuedBy(merkleRoot)` on
 *      THAT clone -> `IssuerRegistry.isWhitelistedFor` against the app's OWN bundled registry. This is
 *      what catches a forged `issuer` block, which sits outside the Merkle root and therefore passes
 *      pillars 1 and 2 unchanged.
 *
 * Every pillar is tri-state and none may be skipped: a pillar that does not resolve yields an
 * indeterminate verdict, never a pass.
 *
 * The record is stored locally under the matching pet, grouped by recordType.
 */
object RecordImporter {

    data class ImportResult(
        val ok: Boolean,
        val verdict: String,          // "VALID" / "INVALID" / "UNVERIFIED"
        val detail: String,
        val credential: Credential?,
    )

    suspend fun import(
        req: QrPayload.ImportRecord,
        issuerRegistry: String,
        issuerFactory: String,
        rpcUrl: String = RoaxRpc.DEFAULT_RPC,
    ): ImportResult {
        // Legacy: fetch the wrapped doc from GET <host>/records/{recordId} with the Bearer record-JWT.
        val url = "${req.host}/records/${req.recordId}"
        val resp = try {
            Http.getJson(url, bearer = req.jwt)
        } catch (e: Exception) {
            return ImportResult(false, "UNVERIFIED", "fetch failed: ${e.message}", null)
        }
        if (!resp.ok) {
            return ImportResult(false, "UNVERIFIED", "GET $url -> ${resp.code}: ${resp.body.take(120)}", null)
        }
        return verifyAndBuild(resp.body, req.recordId, issuerRegistry, issuerFactory, rpcUrl)
    }

    /**
     * Preferred path: resolve a SHORT one-time share token at GET <host>/r/<token> (no Bearer). The
     * server consumes the token (one-time) and returns the wrapped doc; downstream verification is
     * identical to the legacy record-JWT path (integrity FFI + on-chain isValid + store under pet).
     */
    suspend fun import(
        req: QrPayload.ImportRecordToken,
        issuerRegistry: String,
        issuerFactory: String,
        rpcUrl: String = RoaxRpc.DEFAULT_RPC,
    ): ImportResult {
        val url = "${req.host}/r/${req.token}"
        val resp = try {
            Http.getJson(url)
        } catch (e: Exception) {
            return ImportResult(false, "UNVERIFIED", "fetch failed: ${e.message}", null)
        }
        if (!resp.ok) {
            return ImportResult(false, "UNVERIFIED", "GET $url -> ${resp.code}: ${resp.body.take(120)}", null)
        }
        return verifyAndBuild(resp.body, req.token, issuerRegistry, issuerFactory, rpcUrl)
    }

    /**
     * Shared verification + credential build for both import paths. `issuerRegistry` is THIS APP's
     * configured `IssuerRegistry` (bundled `roax.json`), never an address named by the document —
     * otherwise the whitelist pillar would let an attacker answer his own question.
     */
    private suspend fun verifyAndBuild(
        wrappedJson: String,
        fallbackId: String,
        issuerRegistry: String,
        issuerFactory: String,
        rpcUrl: String,
    ): ImportResult {
        val doc = try {
            WrappedDoc(wrappedJson)
        } catch (e: Exception) {
            return ImportResult(false, "UNVERIFIED", "bad wrapped doc: ${e.message}", null)
        }

        // 2. INTEGRITY pillar (offline, Rust FFI).
        val integrity = try {
            verifyIntegrity(wrappedJson)   // "VALID" / "INVALID"
        } catch (e: Exception) {
            "INVALID"
        }

        // 3. ISSUANCE pillar (on-chain isValid via ROAX RPC).
        val onchain = RoaxRpc.isValid(rpcUrl, doc.documentStore, doc.merkleRoot)

        // NO FAIL-OPEN: an unreachable RPC means we did not check, which is NOT the same as checking
        // and finding the record good. It records UNVERIFIED with the reason, never VALID.
        val integrityOk = integrity == "VALID"
        // `var` only so the issuer-whitelist pillar below can tighten it; the mapping arms themselves
        // are left exactly as the refresh-from-chain work wrote them.
        var (verdict, reason) = when {
            !integrityOk ->
                "INVALID" to "the record's contents do not match its signed Merkle root"
            onchain is RoaxRpc.Result.Invalid -> "INVALID" to "revoked or not anchored on ROAX"
            onchain is RoaxRpc.Result.Valid -> "VALID" to "anchored on ROAX and not revoked"
            else -> "UNVERIFIED" to
                "could not reach the chain (${(onchain as RoaxRpc.Result.Unknown).reason})"
        }

        // 4. ISSUER-WHITELIST pillar (on-chain, MANDATORY).
        //
        // Integrity and issuance together still accept a forged authority: the `issuer` block is
        // outside the Merkle root, so relabelling the issuer — or pointing `documentStore` at a
        // contract that returns true from `isValid` — passes both. This pillar resolves the issuing
        // clone from the app's OWN factory, then asks THAT clone what record type it holds and who
        // issued the root, and checks that signer against the app's own bundled registry.
        val whitelist = RoaxRpc.issuerWhitelistPillar(
            rpcUrl, issuerRegistry, issuerFactory, doc.documentStore, doc.merkleRoot, doc.recordType,
        )
        verdict = foldIssuerWhitelist(verdict, whitelist)

        val chainNote = when (onchain) {
            is RoaxRpc.Result.Valid -> "on-chain isValid: yes"
            is RoaxRpc.Result.Invalid -> "on-chain isValid: NO (revoked/not anchored)"
            is RoaxRpc.Result.Unknown -> "on-chain isValid: unknown (${onchain.reason})"
        }
        val whitelistNote = when (whitelist) {
            is RoaxRpc.Result.Valid -> "issuer whitelist: yes"
            is RoaxRpc.Result.Invalid -> "issuer whitelist: NO (signer not authorized for this record type)"
            is RoaxRpc.Result.Unknown -> "issuer whitelist: unresolved (${whitelist.reason})"
        }
        val detail =
            "integrity: $integrity · $chainNote · $whitelistNote · issuer ${doc.issuerDomain.ifBlank { doc.issuerName }}"

        val group = CredentialGroup.fromRecordType(doc.recordType)
        val dogTagId = doc.dogTagId.ifBlank { fallbackId }
        val cred = Credential(
            id = fallbackId,
            dogTagId = dogTagId,
            group = group,
            recordType = doc.recordType.ifBlank { "RECORD" },
            title = doc.displayTitle(),
            subtitle = doc.recordType.ifBlank { "Imported record" },
            issuer = doc.issuerName,
            issuedOn = "",
            credentialRoot = doc.merkleRoot,
            verdict = verdict,
            wrappedDocJson = wrappedJson,
            importedAt = Stamp.now(),
            lastCheckedAt = Stamp.now(),
            verdictReason = reason,
        )
        return ImportResult(verdict != "INVALID", verdict, detail, cred)
    }

    /**
     * Fold the issuer-whitelist pillar into the verdict the integrity + issuance pillars produced.
     *
     * Deliberately a separate, MONOTONE step rather than another arm of the issuance mapping: it can
     * only make a verdict stricter, never looser, so it composes with whatever that mapping decides —
     * including a stricter future mapping (e.g. once an unresolved chain read stops yielding VALID).
     *
     * - [RoaxRpc.Result.Valid]   the issuing signer is authorized: the verdict stands as-is.
     * - [RoaxRpc.Result.Invalid] resolved, and that signer may NOT issue this record type: a real
     *   authenticity failure, so INVALID.
     * - [RoaxRpc.Result.Unknown] the pillar did not resolve. An unanswered check is never a passed
     *   check, so a would-be VALID degrades to UNVERIFIED; anything already worse stands.
     */
    fun foldIssuerWhitelist(verdict: String, pillar: RoaxRpc.Result): String =
        IssuerWhitelist.fold(verdict, "", pillar).first
}

/**
 * THE issuer-whitelist decision. Both surfaces that produce a verdict — import and refresh — go
 * through here, deliberately.
 *
 * They did not always. [CredentialRefresher] derived its own verdict from `isValid` alone, with no
 * whitelist pillar, so the first refresh UPGRADED a credential that import had correctly recorded as
 * UNVERIFIED back to VALID. The feature added so a revocation could reach the user was, on that path,
 * laundering an unverified credential into a trusted one — worse than the original fail-open, which
 * at least never overwrote a correct negative with a positive. Two parallel implementations of one
 * verdict rule is how that happened, so there is now exactly one.
 *
 * Mirrors iOS `IssuerWhitelist` in RecordImporter.swift.
 */
object IssuerWhitelist {

    /**
     * Fold the pillar into a (verdict, reason) pair. MONOTONE: strictly downward, never upward, so it
     * can only tighten what the integrity + issuance pillars already decided. The reason moves WITH
     * the verdict — a degraded verdict still carrying "anchored on ROAX and not revoked" would be an
     * over-claiming explanation stapled to a correct verdict.
     */
    fun fold(verdict: String, reason: String, pillar: RoaxRpc.Result): Pair<String, String> =
        when (pillar) {
            is RoaxRpc.Result.Valid -> verdict to reason
            is RoaxRpc.Result.Invalid ->
                "INVALID" to "the address that issued this record is not authorised to issue this record type"
            is RoaxRpc.Result.Unknown ->
                if (verdict == "VALID") {
                    "UNVERIFIED" to "could not establish who issued this record (${pillar.reason})"
                } else {
                    verdict to reason
                }
        }

    /**
     * Resolve the pillar on-chain and fold it, in one call. [pillar] is injectable so a test can
     * drive the rule without a live RPC.
     */
    suspend fun evaluate(
        verdict: String,
        reason: String,
        rpcUrl: String,
        roax: RoaxConfig,
        documentStore: String,
        root: String,
        recordType: String,
        pillar: RoaxRpc.Result? = null,
    ): Pair<String, String> {
        val resolved = pillar ?: RoaxRpc.issuerWhitelistPillar(
            rpcUrl, roax.issuerRegistry, roax.dogTagIssuerFactory, documentStore, root, recordType,
        )
        return fold(verdict, reason, resolved)
    }
}
