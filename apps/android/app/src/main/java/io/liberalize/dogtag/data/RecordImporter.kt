package io.liberalize.dogtag.data

import io.liberalize.dogtag.net.Http
import io.liberalize.dogtag.net.RoaxRpc
import io.liberalize.dogtag.qr.QrPayload
import uniffi.dogtag_standard.dogTagIdFieldHex
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
        return verifyAndBuild(resp.body, fallbackId = req.recordId, rpcUrl = rpcUrl)
    }

    /**
     * Preferred path: resolve a SHORT one-time share token at GET <host>/r/<token> (no Bearer). The
     * server consumes the token (one-time) and returns the wrapped doc; downstream verification is
     * identical to the legacy record-JWT path (integrity FFI + on-chain isValid + store under pet).
     */
    suspend fun import(req: QrPayload.ImportRecordToken, rpcUrl: String = RoaxRpc.DEFAULT_RPC): ImportResult {
        val url = "${req.host}/r/${req.token}"
        val resp = try {
            Http.getJson(url)
        } catch (e: Exception) {
            return ImportResult(false, "UNVERIFIED", "fetch failed: ${e.message}", null)
        }
        if (!resp.ok) {
            return ImportResult(false, "UNVERIFIED", "GET $url -> ${resp.code}: ${resp.body.take(120)}", null)
        }
        return verifyAndBuild(resp.body, fallbackId = req.token, rpcUrl = rpcUrl)
    }

    /** Shared verification + credential build for both import paths. */
    private suspend fun verifyAndBuild(wrappedJson: String, fallbackId: String, rpcUrl: String): ImportResult {
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

    /** The outcome of polling the chain for the async SBT mint. */
    sealed class MintPoll {
        /** The SBT mint landed: profileRoot(dogTagId) == root AND ownerOf(dogTagId) == wallet. */
        object Confirmed : MintPoll()
        /** Polled for the full window without observing both anchors (the mint did not land in time). */
        object Timeout : MintPoll()
    }

    /**
     * Poll the DogTagSBT for the ASYNC mint after a bind. The vet responds immediately (status "minting")
     * and mints in the background (~12-24s on ROAX), so the on-chain anchors appear only after a few
     * blocks. We poll `profileRoot(dogTagId)` + `ownerOf(dogTagId)` every [intervalMs] for up to
     * [timeoutMs], succeeding once BOTH match (case-insensitive). A miss is retried — NOT a hard fail.
     */
    suspend fun pollSbtMint(
        dogTagId: String,
        expectedRoot: String,
        walletAddress: String,
        dogTagSbt: String,
        rpcUrl: String = RoaxRpc.DEFAULT_RPC,
        intervalMs: Long = 3000,
        timeoutMs: Long = 120_000,
        onTick: (suspend () -> Unit)? = null,
    ): MintPoll {
        // The SBT is minted under the canonical on-chain id `field_of_value(dogTagId)` (matching the
        // issuance/export path), so the anchor reads must use the field-hashed id, not the raw handle.
        // `dogTagIdFieldHex` returns 0x..32-byte hex; RoaxRpc encodes the tokenId from a decimal string,
        // so convert back to decimal to match the existing (decimal) id convention.
        val onchainDogTagId = java.math.BigInteger(
            dogTagIdFieldHex(dogTagId).removePrefix("0x"), 16,
        ).toString()
        val deadline = System.currentTimeMillis() + timeoutMs
        while (System.currentTimeMillis() < deadline) {
            val onchainRoot = RoaxRpc.profileRoot(rpcUrl, dogTagSbt, onchainDogTagId)
            val onchainOwner = RoaxRpc.ownerOf(rpcUrl, dogTagSbt, onchainDogTagId)
            val rootOk = onchainRoot != null && onchainRoot.equals(expectedRoot, ignoreCase = true)
            val ownerOk = onchainOwner != null && onchainOwner.equals(walletAddress, ignoreCase = true)
            if (rootOk && ownerOk) return MintPoll.Confirmed
            onTick?.invoke()
            kotlinx.coroutines.delay(intervalMs)
        }
        return MintPoll.Timeout
    }

    /**
     * The vet-issues-the-dog-tag (DOG_PROFILE) verify+store branch. Unlike the import path above, a
     * DOG_PROFILE is anchored in the DogTagSBT — NOT in a DogTagIssuer clone — so the issuance pillar is:
     *
     *   1. INTEGRITY (offline, Rust FFI): `verifyIntegrity(wrappedDocJson)` == "VALID", and the doc's
     *      `signature.merkleRoot` must equal the bind-returned `expectedRoot`.
     *   2. SBT ANCHOR (on-chain): `DogTagSBT.profileRoot(dogTagId)` == `expectedRoot`, AND
     *      `DogTagSBT.ownerOf(dogTagId)` == `walletAddress`.
     *
     * This does a SINGLE on-chain check — with the async-mint flow the caller should first await
     * [pollSbtMint] (which retries the anchor read) so this check sees the landed mint. Returns the built
     * Credential (stored by the caller under the same path as import) + a verdict.
     */
    suspend fun verifyIssuedDogTag(
        wrappedDocJson: String,
        dogTagId: String,
        expectedRoot: String,
        walletAddress: String,
        dogTagSbt: String,
        rpcUrl: String = RoaxRpc.DEFAULT_RPC,
    ): ImportResult {
        val doc = try {
            WrappedDoc(wrappedDocJson)
        } catch (e: Exception) {
            return ImportResult(false, "UNVERIFIED", "bad wrapped doc: ${e.message}", null)
        }

        // 1. INTEGRITY (offline) + root consistency (doc.merkleRoot == bind root).
        val integrity = try { verifyIntegrity(wrappedDocJson) } catch (e: Exception) { "INVALID" }
        val integrityOk = integrity == "VALID"
        val rootMatchesDoc = expectedRoot.isNotBlank() &&
            doc.merkleRoot.equals(expectedRoot, ignoreCase = true)

        // 2. SBT ANCHOR (on-chain) — profileRoot(id) == root AND ownerOf(id) == wallet, where the id is
        // the canonical `field_of_value(dogTagId)` the SBT was minted under (decimal, to match RoaxRpc's
        // existing tokenId encoding). The raw `dogTagId` is still used for the credential/record fields.
        val onchainDogTagId = java.math.BigInteger(
            dogTagIdFieldHex(dogTagId).removePrefix("0x"), 16,
        ).toString()
        val onchainRoot = RoaxRpc.profileRoot(rpcUrl, dogTagSbt, onchainDogTagId)
        val onchainOwner = RoaxRpc.ownerOf(rpcUrl, dogTagSbt, onchainDogTagId)
        val rootOk = onchainRoot != null && onchainRoot.equals(expectedRoot, ignoreCase = true)
        val ownerOk = onchainOwner != null && onchainOwner.equals(walletAddress, ignoreCase = true)
        val rpcReached = onchainRoot != null && onchainOwner != null

        val verdict = when {
            !integrityOk || !rootMatchesDoc -> "INVALID"
            rpcReached && (!rootOk || !ownerOk) -> "INVALID"
            rpcReached -> "VALID"
            else -> "VALID"   // integrity passed; chain unreachable -> accept with caveat
        }

        val chainNote = when {
            !rpcReached -> "SBT verify: unknown (RPC unreachable)"
            rootOk && ownerOk -> "SBT verify: profileRoot + ownerOf match"
            !rootOk -> "SBT verify: profileRoot MISMATCH"
            else -> "SBT verify: ownerOf MISMATCH ($onchainOwner)"
        }
        val detail = "integrity: $integrity · $chainNote · issuer ${doc.issuerDomain.ifBlank { doc.issuerName }}"

        val cred = Credential(
            id = dogTagId.ifBlank { expectedRoot },
            dogTagId = dogTagId,
            group = CredentialGroup.fromRecordType(doc.recordType),
            recordType = doc.recordType.ifBlank { "DOG_PROFILE" },
            title = doc.displayTitle(),
            subtitle = doc.recordType.ifBlank { "Dog tag" },
            issuer = doc.issuerName,
            issuedOn = "",
            credentialRoot = doc.merkleRoot.ifBlank { expectedRoot },
            verdict = verdict,
            wrappedDocJson = wrappedDocJson,
        )
        return ImportResult(verdict != "INVALID", verdict, detail, cred)
    }
}
