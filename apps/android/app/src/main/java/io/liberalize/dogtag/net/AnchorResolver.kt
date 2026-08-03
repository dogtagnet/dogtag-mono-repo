package io.liberalize.dogtag.net

/**
 * Client-side discovery anchor resolution for the owner-hidden protocol.
 *
 * Before the app acts on a platform's owner-hidden session it must pin the
 * platform's CLAIMS (`{protocolVersion, chainId, verificationRegistry, purpose}` from the resolve
 * GET) against the dogtag-owned TRUST ANCHOR read from `ProtocolRegistry` on-chain — otherwise a
 * lying platform could steer an owner-hidden proof onto an attacker registry/chain. The actual
 * fail-closed comparison is the SHIPPED, cross-platform `validateDiscovery` UniFFI binding
 * (`dogtag_standard::discovery::validate`); this file does the two jobs that binding leaves to the
 * caller (its own docs: "RESOLVING the anchor is the CALLER's job"):
 *
 * It decodes the two `ProtocolRegistry` getter returns — [decodeDiscoverySet] / [decodeArtifactSet]
 *      — into plain records. Kept PURE (no FFI, no chain I/O) so they compile into the host-less JVM
 *      unit tests and are pinned against fixed hex vectors.
 *
 * The FFI glue that builds a `TrustedAnchor` from these records and calls `validateDiscovery` lives in
 * `ScanScreen` (it needs the FFI module, which this file deliberately does not import).
 *
 * Mirrored, with the same split, in `apps/ios/DogTag/AnchorResolver.swift`.
 */
object AnchorResolver {
    /** The protocol constants the app supplies to the anchor (the on-chain records carry only
     * the keccak ids; `validateDiscovery` checks `keccak256(string) == id` for both axes). These MUST
     * match `ProtocolVersions.sol` / `dogtag_prover::artifact` — a drift is a fail-closed refusal. */
    const val PROTOCOL_VERSION = "dogtag-levelb/1"
    const val ARTIFACT_SET = "dogtag-levelb-artifacts/1"
    const val CIRCUIT_ID = "consent.circom/DogTagConsent(6)"


    /** The fields of `ProtocolRegistry.DiscoverySet` an app-side anchor needs. */
    data class DiscoverySetRecord(
        val discoverySetId: String, // 0x-hex bytes32
        /** `DogTagIssuerFactory` — where a provider's clone comes from AND the write-once root index. */
        val factory: String, // 0x-hex address (lowercased)
        val verificationRegistry: String, // 0x-hex address (lowercased)
        /** The `ProviderRegistry` authority core — also the root of the resolver layer. */
        val providerRegistry: String, // 0x-hex address (lowercased)
        val circuitId: String, // 0x-hex bytes32
        val active: Boolean,
    ) {
        /** The address `rootIssuer`/`isClone` are read from. There is ONE launch set and no earlier
         * generation to bridge, so the factory IS the root index — `VerificationRegistryConsent` pins
         * it in its immutable `rootIndex` slot, and `ProtocolRegistry.DiscoverySet.factory` carries
         * that same address. Exposed under its own name because the QUESTION differs ("where does a
         * clone come from" vs "what resolves an anchored root"), and a future generation could
         * separate them again. */
        val rootIndex: String get() = factory
    }

    /** The fields of `ProtocolRegistry.ArtifactSet` an app-side anchor needs. `minAppVersion` is the
     * only dynamic-string member we decode (`artifactBaseUrl` is not needed on-device: we bundle the
     * artifacts, we do not fetch them). */
    data class ArtifactSetRecord(
        val artifactSetId: String, // 0x-hex bytes32
        val minAppVersion: String, // decoded semver string
        val active: Boolean,
    )

    // ---- ABI decoding (pure) ----

    /** Split raw ABI return hex (no 0x) into 32-byte words, or null if not a whole number of words. */
    private fun words(hex: String): List<String>? {
        val h = hex.removePrefix("0x")
        if (h.length % 64 != 0) return null
        return (0 until h.length step 64).map { h.substring(it, it + 64) }
    }

    private fun addr(word: String): String = "0x" + word.takeLast(40).lowercase()
    private fun b32(word: String): String = "0x" + word.lowercase()
    private fun boolOf(word: String): Boolean = word.any { it != '0' }

    /** A 32-byte word as an Int byte offset/length (small values only; the low 8 bytes fit an Int). */
    private fun intOf(word: String): Int? = word.takeLast(16).toLongOrNull(16)?.toInt()

    /**
     * Decode `ProtocolRegistry.getDiscoverySet` — a STATIC tuple, so the return is 9 inline words:
     * `[discoverySetId, factory, verificationRegistry, sbt, verifier, providerRegistry, circuitId,
     * publishedAt, active]`. Kept: `discoverySetId`(0), `factory`(1), `verificationRegistry`(2),
     * `providerRegistry`(5), `circuitId`(6), `active`(8).
     *
     * The arity is required EXACTLY, not as a lower bound. A static tuple encodes one word per member,
     * so 9 is the only width this record can have, and an inexact check is how a differently-shaped
     * return gets decoded at the wrong indices into a plausible-looking live record. This app shipped
     * a decoder demanding 10 words (a design with a separate `rootIndex`) against a contract that
     * returns 9 — that guard is why the mismatch surfaced as a refusal rather than as an anchor naming
     * whatever sat at those offsets.
     */
    fun decodeDiscoverySet(hex: String): DiscoverySetRecord? {
        val w = words(hex) ?: return null
        if (w.size != 9) return null
        return DiscoverySetRecord(
            discoverySetId = b32(w[0]),
            factory = addr(w[1]),
            verificationRegistry = addr(w[2]),
            providerRegistry = addr(w[5]),
            circuitId = b32(w[6]),
            active = boolOf(w[8]),
        )
    }

    /**
     * Decode `ProtocolRegistry.getActiveArtifactSet` — a DYNAMIC tuple (two `string` members), so the
     * return is a leading offset word pointing at the tuple, whose head is 9 words:
     * `[artifactSetId, zkeySha, witnessMobileSha, witnessR1csSha, witnessWasmSha, artifactBaseUrl↗,
     * minAppVersion↗, publishedAt, active]`. We read `artifactSetId`(head 0) and `active`(head 8)
     * directly, and FOLLOW the `minAppVersion` offset (head 6, relative to the tuple start) to its
     * `[length][bytes]` tail. `artifactBaseUrl` (head 5) is ignored — not needed on-device.
     */
    fun decodeArtifactSet(hex: String): ArtifactSetRecord? {
        val w = words(hex) ?: return null
        if (w.isEmpty()) return null
        val tupleByte = intOf(w[0]) ?: return null
        if (tupleByte % 32 != 0) return null
        val tupleWord = tupleByte / 32
        if (w.size < tupleWord + 9) return null
        val artifactSetId = b32(w[tupleWord + 0])
        val active = boolOf(w[tupleWord + 8])
        val relOff = intOf(w[tupleWord + 6]) ?: return null
        if (relOff % 32 != 0) return null
        val strWord = tupleWord + relOff / 32
        if (w.size <= strWord) return null
        val len = intOf(w[strWord]) ?: return null
        val minAppVersion = readString(w, strWord, len) ?: return null
        return ArtifactSetRecord(artifactSetId, minAppVersion, active)
    }

    /** Read a UTF-8 string whose ABI `[length]` word is at `words[lenWordIdx]` and whose bytes follow.
     * Fails closed on any bounds/UTF-8 error — a malformed `minAppVersion` must never read as new. */
    private fun readString(words: List<String>, lenWordIdx: Int, byteLen: Int): String? {
        if (byteLen < 0) return null
        if (byteLen == 0) return ""
        val wordsNeeded = (byteLen + 31) / 32
        val firstDataWord = lenWordIdx + 1
        if (words.size < firstDataWord + wordsNeeded) return null
        val hexBytes = (0 until wordsNeeded).joinToString("") { words[firstDataWord + it] }
        val hexCount = byteLen * 2
        if (hexBytes.length < hexCount) return null
        val bytes = ByteArray(byteLen)
        var i = 0
        while (i < byteLen) {
            val b = hexBytes.substring(i * 2, i * 2 + 2).toIntOrNull(16) ?: return null
            bytes[i] = b.toByte()
            i += 1
        }
        return try {
            String(bytes, Charsets.UTF_8)
        } catch (e: Exception) {
            null
        }
    }
}
