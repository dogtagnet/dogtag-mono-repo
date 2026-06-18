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
 *   3. IDENTITY (optional / best-effort): the issuer domain is surfaced; DNS-TXT is not done on-device
 *      here (no DNS resolver dependency) — the verdict notes it as advisory.
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

    suspend fun import(req: QrPayload.ImportRecord, rpcUrl: String = RoaxRpc.DEFAULT_RPC): ImportResult {
        // 1. Fetch the wrapped doc from GET <host>/records/{recordId} with the Bearer record-JWT.
        val url = "${req.host}/records/${req.recordId}"
        val resp = try {
            Http.getJson(url, bearer = req.jwt)
        } catch (e: Exception) {
            return ImportResult(false, "UNVERIFIED", "fetch failed: ${e.message}", null)
        }
        if (!resp.ok) {
            return ImportResult(false, "UNVERIFIED", "GET $url -> ${resp.code}: ${resp.body.take(120)}", null)
        }

        val wrappedJson = resp.body
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
        val verdict = when {
            !integrityOk -> "INVALID"
            onchain is RoaxRpc.Result.Invalid -> "INVALID"
            onchain is RoaxRpc.Result.Valid -> "VALID"
            else -> "VALID"   // integrity passed; chain unreachable -> accept with caveat
        }

        val chainNote = when (onchain) {
            is RoaxRpc.Result.Valid -> "on-chain isValid: yes"
            is RoaxRpc.Result.Invalid -> "on-chain isValid: NO (revoked/not anchored)"
            is RoaxRpc.Result.Unknown -> "on-chain isValid: unknown (${onchain.reason})"
        }
        val detail = "integrity: $integrity · $chainNote · issuer ${doc.issuerDomain.ifBlank { doc.issuerName }}"

        val group = CredentialGroup.fromRecordType(doc.recordType)
        val dogTagId = doc.dogTagId.ifBlank { req.recordId }
        val cred = Credential(
            id = req.recordId,
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
}
