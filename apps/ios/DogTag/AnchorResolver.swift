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

    /// The GENERATION-2 discovery key, published to `ProtocolRegistryV2` (`ProtocolVersionsV2.sol`).
    ///
    /// The artifact key and the circuit id are NOT versioned alongside it: generation 2 rotates the
    /// factory, the verification registry, the authority core and the root index, and rotates no proving
    /// artifact — the circuit, the ceremony and all four pins are unchanged. Bumping `artifactSet` would
    /// also make an old build fail with a stitched-anchor coherence error instead of the `minAppVersion`
    /// refusal that actually tells the holder to update.
    static let protocolVersionV2 = "dogtag-levelb/2"

    /// The fields of `ProtocolRegistry.ContractSet` an app-side anchor needs. `active` →
    /// `TrustedAnchor.contractSetActive`; `verificationRegistry` is the anti-redirect address;
    /// `contractSetId` → `TrustedAnchor.versionId`.
    struct ContractSetRecord: Equatable {
        let contractSetId: String // 0x-hex bytes32
        let verificationRegistry: String // 0x-hex address (lowercased)
        let circuitId: String // 0x-hex bytes32
        let active: Bool
    }

    /// The fields of `ProtocolRegistry.ArtifactSet` an app-side anchor needs. `minAppVersion` is the
    /// only dynamic-string member we decode (`artifactBaseUrl` is not needed on-device: we bundle the
    /// artifacts, we do not fetch them). `active` → `TrustedAnchor.artifactSetActive`.
    struct ArtifactSetRecord: Equatable {
        let artifactSetId: String // 0x-hex bytes32
        let minAppVersion: String // decoded semver string
        let active: Bool
    }

    /// The fields of `ProtocolRegistryV2.DiscoverySet` an app-side anchor needs — the generation-1 set
    /// plus the two members generation 2 adds.
    ///
    /// A separate type from `ContractSetRecord` on purpose, decoded by a separate function: the two
    /// on-chain records have different arities and different member positions, so one record type
    /// carrying optional extras would let a generation-1 return populate a generation-2 record with
    /// whatever happened to sit at those indices.
    struct DiscoverySetRecord: Equatable {
        let discoverySetId: String // 0x-hex bytes32
        let verificationRegistry: String // 0x-hex address (lowercased)
        /// The `ProviderRegistry` authority core — also the root of the resolver layer.
        let providerRegistry: String // 0x-hex address (lowercased)
        /// The `CloneProvenanceRouter`. NOT the factory: reading `rootIssuer`/`isClone` from the factory
        /// resolves only that generation's roots and silently misses every earlier one.
        let rootIndex: String // 0x-hex address (lowercased)
        let circuitId: String // 0x-hex bytes32
        let active: Bool
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

    /// Decode `ProtocolRegistry.getContractSet` — a STATIC tuple, so the return is 8 inline words:
    /// `[contractSetId, factory, verificationRegistry, sbt, verifier, circuitId, publishedAt, active]`.
    /// Only `contractSetId`(0), `verificationRegistry`(2), `circuitId`(5), `active`(7) are kept.
    ///
    /// The arity is required EXACTLY, not as a lower bound. A static tuple's encoding is one word per
    /// member, so 8 is the only width this record can have — and a `>= 8` check would happily decode the
    /// 10-word generation-2 record at generation-1 indices, reading `providerRegistry` as `circuitId` and
    /// `publishedAt` as `active`, which yields a plausible-looking live record for the wrong generation.
    /// Refusing a width this record cannot have is a cheap structural guard against exactly that.
    static func decodeContractSet(_ hex: String) -> ContractSetRecord? {
        guard let w = words(hex), w.count == 8 else { return nil }
        return ContractSetRecord(
            contractSetId: b32(w[0]),
            verificationRegistry: addr(w[2]),
            circuitId: b32(w[5]),
            active: boolOf(w[7]))
    }

    /// Decode `ProtocolRegistryV2.getDiscoverySet` — also a STATIC tuple, 10 inline words:
    /// `[discoverySetId, factory, verificationRegistry, sbt, verifier, providerRegistry, rootIndex,
    /// circuitId, publishedAt, active]`. Kept: `discoverySetId`(0), `verificationRegistry`(2),
    /// `providerRegistry`(5), `rootIndex`(6), `circuitId`(7), `active`(9).
    ///
    /// The arity is required exactly, for the mirror of the reason above: a generation-1 return decoded
    /// here would read `publishedAt` as `circuitId` and run off the end for `active`.
    static func decodeDiscoverySet(_ hex: String) -> DiscoverySetRecord? {
        guard let w = words(hex), w.count == 10 else { return nil }
        return DiscoverySetRecord(
            discoverySetId: b32(w[0]),
            verificationRegistry: addr(w[2]),
            providerRegistry: addr(w[5]),
            rootIndex: addr(w[6]),
            circuitId: b32(w[7]),
            active: boolOf(w[9]))
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
