import Foundation

/// Client-side owner-hidden discovery-anchor resolution.
///
/// Before the app acts on a platform's owner-hidden session it must pin the
/// platform's CLAIMS (`{protocolVersion, chainId, verificationRegistry, purpose}` from the resolve
/// GET) against the dogtag-owned TRUST ANCHOR read from `ProtocolRegistry` on-chain — otherwise a
/// lying platform could steer an owner-hidden proof onto an attacker registry/chain. The actual
/// fail-closed comparison is the SHIPPED, cross-platform `validateDiscovery` UniFFI binding
/// (`dogtag_standard::discovery::validate`); this file does the two jobs that binding leaves to the
/// caller (its own docs: "RESOLVING the anchor is the CALLER's job"):
///
///   1. DECODING the two `ProtocolRegistry` getter returns — [`decodeContractSet`] /
///      [`decodeArtifactSet`] — into plain records. Kept PURE (Foundation only, no FFI, no chain I/O)
///      so they compile into the host-less test bundle and are unit-tested against fixed hex vectors.
///
/// The FFI glue that builds a `TrustedAnchor` from these records and calls `validateDiscovery` lives
/// in `ScanScreen` (it needs the FFI module, which this file deliberately does not import).
///
/// Mirrored, with the same split, in `apps/android/.../net/AnchorResolver.kt`.
enum AnchorResolver {
    /// Protocol constants the app supplies to the anchor (the on-chain records carry only
    /// the keccak ids; `validateDiscovery` checks `keccak256(string) == id` for both axes). These MUST
    /// match `ProtocolVersions.sol` / `dogtag_prover::artifact` — a drift is a fail-closed refusal.
    static let protocolVersion = "dogtag-levelb/1"
    static let artifactSet = "dogtag-levelb-artifacts/1"
    static let circuitId = "consent.circom/DogTagConsent(6)"


    /// The fields of `ProtocolRegistry.ArtifactSet` an app-side anchor needs. `minAppVersion` is the
    /// only dynamic-string member we decode (`artifactBaseUrl` is not needed on-device: we bundle the
    /// artifacts, we do not fetch them). `active` → `TrustedAnchor.artifactSetActive`.
    struct ArtifactSetRecord: Equatable {
        let artifactSetId: String // 0x-hex bytes32
        let minAppVersion: String // decoded semver string
        let active: Bool
    }

    /// The fields of `ProtocolRegistryV2.DiscoverySet` an app-side anchor needs — the generation-1 set
    /// needs.
    struct DiscoverySetRecord: Equatable {
        let discoverySetId: String // 0x-hex bytes32
        /// `DogTagIssuerFactory` — where a provider's clone comes from AND the write-once root index.
        let factory: String // 0x-hex address (lowercased)
        let verificationRegistry: String // 0x-hex address (lowercased)
        /// The `ProviderRegistry` authority core — also the root of the resolver layer.
        let providerRegistry: String // 0x-hex address (lowercased)
        let circuitId: String // 0x-hex bytes32
        let active: Bool

        /// The address `rootIssuer`/`isClone` are read from. There is ONE launch set and no earlier
        /// generation to bridge, so the factory IS the root index — `VerificationRegistryConsent`
        /// pins it in its immutable `rootIndex` slot, and `ProtocolRegistry.DiscoverySet.factory`
        /// carries that same address. It is exposed as its own name because the QUESTION differs
        /// ("where does a clone come from" vs "what resolves an anchored root"), and a future
        /// generation could separate them again.
        var rootIndex: String { factory }
    }

    // MARK: - ABI decoding (pure)

    /// Split raw ABI return hex (no `0x`) into 32-byte words. Returns nil if the length is not a
    /// whole number of words — a malformed return fails closed rather than decoding garbage.
    private static func words(_ hex: String) -> [String]? {
        let h = hex.hasPrefix("0x") ? String(hex.dropFirst(2)) : hex
        guard h.count % 64 == 0 else { return nil }
        var out: [String] = []
        var i = h.startIndex
        while i < h.endIndex {
            let j = h.index(i, offsetBy: 64)
            out.append(String(h[i..<j]))
            i = j
        }
        return out
    }

    /// A 32-byte word (last 20 bytes) as a lowercased `0x` address.
    private static func addr(_ word: String) -> String { "0x" + word.suffix(40).lowercased() }
    /// A 32-byte word as a `0x` bytes32.
    private static func b32(_ word: String) -> String { "0x" + word.lowercased() }
    /// A 32-byte word's low byte as a bool (ABI encodes bool as 0/1 in the last byte).
    private static func boolOf(_ word: String) -> Bool { word.contains { $0 != "0" } }
    /// A 32-byte word as an Int byte offset (small values only — offsets/lengths fit in Int here).
    private static func intOf(_ word: String) -> Int? { Int(word.suffix(16), radix: 16) }

    /// Decode `ProtocolRegistry.getDiscoverySet` — a STATIC tuple, so the return is 9 inline words:
    /// `[discoverySetId, factory, verificationRegistry, sbt, verifier, providerRegistry, circuitId,
    /// publishedAt, active]`. Kept: `discoverySetId`(0), `factory`(1), `verificationRegistry`(2),
    /// `providerRegistry`(5), `circuitId`(6), `active`(8).
    ///
    /// The arity is required EXACTLY, not as a lower bound. A static tuple encodes one word per member,
    /// so 9 is the only width this record can have, and an inexact check is how a differently-shaped
    /// return gets decoded at the wrong indices into a plausible-looking live record. This app shipped
    /// a decoder demanding 10 words (a design with a separate `rootIndex`) against a contract that
    /// returns 9 — that guard is why the mismatch surfaced as a refusal rather than as an anchor
    /// naming whatever sat at those offsets.
    static func decodeDiscoverySet(_ hex: String) -> DiscoverySetRecord? {
        guard let w = words(hex), w.count == 9 else { return nil }
        return DiscoverySetRecord(
            discoverySetId: b32(w[0]),
            factory: addr(w[1]),
            verificationRegistry: addr(w[2]),
            providerRegistry: addr(w[5]),
            circuitId: b32(w[6]),
            active: boolOf(w[8]))
    }

    /// Decode `ProtocolRegistry.getActiveArtifactSet` — a DYNAMIC tuple (two `string` members), so the
    /// return is a leading offset word pointing at the tuple, whose head is 9 words:
    /// `[artifactSetId, zkeySha, witnessMobileSha, witnessR1csSha, witnessWasmSha, artifactBaseUrl↗,
    /// minAppVersion↗, publishedAt, active]`. We read `artifactSetId`(head 0) and `active`(head 8)
    /// directly, and FOLLOW the `minAppVersion` offset (head 6, relative to the tuple start) to its
    /// `[length][bytes]` tail. `artifactBaseUrl` (head 5) is ignored — not needed on-device.
    static func decodeArtifactSet(_ hex: String) -> ArtifactSetRecord? {
        guard let w = words(hex), w.count >= 1 else { return nil }
        // Outer offset (word 0) → the tuple starts `tupleWord` words in.
        guard let tupleByte = intOf(w[0]), tupleByte % 32 == 0 else { return nil }
        let tupleWord = tupleByte / 32
        // Head is 9 words at [tupleWord ..< tupleWord+9].
        guard w.count >= tupleWord + 9 else { return nil }
        let head = tupleWord
        let artifactSetId = b32(w[head + 0])
        let active = boolOf(w[head + 8])
        // minAppVersion: head word 6 is a byte offset RELATIVE to the tuple start.
        guard let relOff = intOf(w[head + 6]), relOff % 32 == 0 else { return nil }
        let strWord = tupleWord + relOff / 32
        guard w.count > strWord, let len = intOf(w[strWord]) else { return nil }
        guard let minAppVersion = readString(words: w, at: strWord, byteLen: len) else { return nil }
        return ArtifactSetRecord(artifactSetId: artifactSetId, minAppVersion: minAppVersion, active: active)
    }

    /// Read a UTF-8 string whose ABI `[length]` word is at `words[lenWordIdx]` and whose bytes follow
    /// in the subsequent words (right-padded to a word boundary). Fails closed on any bounds/UTF-8
    /// error — a malformed `minAppVersion` must never read as "new enough".
    private static func readString(words: [String], at lenWordIdx: Int, byteLen: Int) -> String? {
        guard byteLen >= 0 else { return nil }
        if byteLen == 0 { return "" }
        let wordsNeeded = (byteLen + 31) / 32
        let firstDataWord = lenWordIdx + 1
        guard words.count >= firstDataWord + wordsNeeded else { return nil }
        var hexBytes = ""
        for k in 0..<wordsNeeded { hexBytes += words[firstDataWord + k] }
        // Take exactly byteLen bytes (2 hex chars each) off the front.
        let hexCount = byteLen * 2
        guard hexBytes.count >= hexCount else { return nil }
        let slice = hexBytes.prefix(hexCount)
        var bytes = [UInt8]()
        bytes.reserveCapacity(byteLen)
        var i = slice.startIndex
        while i < slice.endIndex {
            let j = slice.index(i, offsetBy: 2)
            guard let b = UInt8(slice[i..<j], radix: 16) else { return nil }
            bytes.append(b)
            i = j
        }
        return String(bytes: bytes, encoding: .utf8)
    }
}
