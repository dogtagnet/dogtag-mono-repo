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
 *   3. ISSUER WHITELIST (on-chain): `issuedBy(merkleRoot)` -> `IssuerRegistry.isWhitelistedFor` against
 *      the app's OWN bundled registry. This is what catches a forged `issuer` block, which sits
 *      outside the Merkle root and therefore passes pillars 1 and 2 unchanged.
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
        return verifyAndBuild(resp.body, req.recordId, issuerRegistry, rpcUrl)
    }

    /**
     * Preferred path: resolve a SHORT one-time share token at GET <host>/r/<token> (no Bearer). The
     * server consumes the token (one-time) and returns the wrapped doc; downstream verification is
     * identical to the legacy record-JWT path (integrity FFI + on-chain isValid + store under pet).
     */
    suspend fun import(
        req: QrPayload.ImportRecordToken,
        issuerRegistry: String,
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
        return verifyAndBuild(resp.body, req.token, issuerRegistry, rpcUrl)
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

        val integrityOk = integrity == "VALID"
        // `var` only so the issuer-whitelist pillar below can tighten it; the mapping itself is
        // deliberately left exactly as-is.
        var verdict = when {
            !integrityOk -> "INVALID"
            onchain is RoaxRpc.Result.Invalid -> "INVALID"
            onchain is RoaxRpc.Result.Valid -> "VALID"
            else -> "VALID"   // integrity passed; chain unreachable -> accept with caveat
        }

        // 4. ISSUER-WHITELIST pillar (on-chain, MANDATORY).
        //
        // Integrity and issuance together still accept a forged authority: the `issuer` block is
        // outside the Merkle root, so relabelling the issuer — or pointing `documentStore` at a
        // contract that returns true from `isValid` — passes both. This pillar asks the chain who
        // actually issued the root and whether that signer is whitelisted for this record type in the
        // app's own bundled registry.
        val whitelist = RoaxRpc.issuerWhitelistPillar(
            rpcUrl, issuerRegistry, doc.documentStore, doc.merkleRoot, doc.recordType,
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
    fun foldIssuerWhitelist(verdict: String, pillar: RoaxRpc.Result): String = when (pillar) {
        is RoaxRpc.Result.Valid -> verdict
        is RoaxRpc.Result.Invalid -> "INVALID"
        is RoaxRpc.Result.Unknown -> if (verdict == "VALID") "UNVERIFIED" else verdict
    }
}
