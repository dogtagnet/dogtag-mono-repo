import Foundation

/// The issuer↔domain binding, resolved on the phone.
///
/// # The three-link chain
///
/// All three links are required, and all three are checked HERE at display time rather than inherited
/// from anything a server or a document says:
///
///  1. **Factory provenance** — `DogTagIssuerFactory.isClone(clone)`. Without it the rest is worthless:
///     anyone can deploy their own contract, claim a domain for it and publish a matching TXT record.
///     DNS would agree, the registry would agree, and none of it would mean anything, because that
///     contract never passed through the KYC-gated `createIssuer`.
///  2. **The on-chain domain claim** — `IssuerDomainRegistry.domainOf(clone)`. Read from the CHAIN, never
///     from the credential: the document's `issuer` block is outside the Merkle root, so a relabelled
///     document verifies. Taking the domain from the chain removes that attack entirely, because a
///     relabelled document cannot move a value it does not carry.
///  3. **The DNS half** — that domain's zone publishes
///     `<clone-address-lowercase>._dogtag.<domain> TXT "<free-form description>"`.
///
/// # Where the DNS lookup happens, and what it costs
///
/// The phone resolves DNS-over-HTTPS DIRECTLY, with no DogTag server in the loop. The alternative —
/// routing the lookup through our own API — would tell US which credential the holder is looking at and
/// when, which is strictly worse for a privacy-sensitive wallet than telling a public resolver a domain
/// name. This also matches the existing precedent: the phone already resolves DoH directly for the
/// pre-disclosure groomer binding (`ScanScreen`).
///
/// The privacy cost, stated plainly: **Cloudflare learns the issuer's domain and this device's IP at the
/// moment a credential is viewed.** It does not learn the credential, the holder, or the dogTagId.
///
/// # No synthesized states
///
/// Every state below is the outcome of a real read or a real read that really failed. Nothing is
/// defaulted, and a surface that renders before resolution completes shows `.pending` rather than
/// guessing — flashing "no domain claimed" and then flipping would be showing a state never observed.
enum IssuerBindingState: Equatable {
    /// The domain publishes a record for this contract. `description` is the domain owner's own words.
    case verified(description: String)
    /// Link 1 failed: this contract did not come from the DogTag factory. Categorically stronger than a
    /// missing DNS record and must never be shown as one.
    case notADogTagIssuer
    /// Links 1 and 2 hold; the domain publishes no record for this address. A NORMAL state.
    case notListed
    /// A real DNS attempt failed. Evidence of nothing.
    case couldNotCheck
    /// Link 1 holds; this issuer has claimed no domain on-chain. The normal day-one state.
    case noDomainClaimed
    /// A prerequisite could not be read at all (nothing configured, or the read failed).
    case unavailable
    /// In flight. Never a resting state.
    case pending
}

/// Where the contract this binding was resolved against came from.
///
/// Not cosmetic: the two carry different authority, and the difference is the whole of the sharper
/// relabelling attack. `rootIssuer` is the factory's write-once record of which clone issued THIS root.
/// `documentClaim` is the credential's own `issuer.documentStore`, which sits outside the Merkle root
/// and can be pointed at any address — including another authority's genuine, factory-deployed clone.
enum IssuerCloneSource: Equatable {
    /// The factory's write-once `rootIssuer[R]`. Authoritative.
    case rootIssuer
    /// The document's own claim, reached only because the factory has no record of this root. Never
    /// authoritative, and always labelled as the fallback it is.
    case documentClaim
}

/// The pure decision "which contract does this binding describe", separated from the I/O so it is
/// testable without a network. See [`IssuerBindingResolver.chooseClone`].
struct IssuerCloneChoice: Equatable {
    /// The contract every link of the chain is then resolved against.
    var address: String = ""
    var source: IssuerCloneSource = .documentClaim
    /// The chain names a DIFFERENT issuing contract than the document does. Reported, never followed.
    var documentStoreDiffers: Bool = false
    /// The `rootIssuer` read did not resolve at all. The document's claim is then unchecked, so the
    /// caller must report "could not read" rather than proceeding on it.
    var readFailed: Bool = false
}

/// A resolved binding plus the provenance needed to be honest about it.
struct IssuerBinding: Equatable {
    var state: IssuerBindingState = .pending
    /// The ON-CHAIN claimed domain that was queried.
    var domain: String = ""
    /// The clone's on-chain `name()` — the only authoritative issuer name.
    var onchainName: String = ""
    /// The contract this binding actually describes, and where that address came from.
    var cloneAddress: String = ""
    var cloneSource: IssuerCloneSource = .documentClaim
    /// The chain's write-once `rootIssuer[R]` names a different contract than the document's
    /// `issuer.documentStore`. Positive evidence the document's claim was swapped.
    var documentStoreDiffers: Bool = false
    /// The block every on-chain read was pinned to. `nil` == the head could not be read, so this answer
    /// is not reproducible and must not imply a block.
    var blockNumber: UInt64?
    /// When the DNS half was observed. DNS has no history, so this is the only timestamp it can carry.
    var checkedAt: Date?

    static let pending = IssuerBinding()

    var isVerified: Bool {
        if case .verified = state { return true }
        return false
    }
}

/// Resolves the binding for a credential's issuer contract.
enum IssuerBindingResolver {
    /// The underscore label that namespaces every DogTag binding record. An underscore label cannot
    /// collide with a real hostname, so the binding can never shadow something the domain owner serves.
    /// Address-label-first mirrors DKIM's `<selector>._domainkey.<domain>`.
    static let label = "_dogtag"

    /// A small in-process cache. A DNS lookup per row render is unacceptable, and a cached entry is a
    /// real prior observation — it keeps its ORIGINAL `checkedAt`, so a surface shows when DNS was really
    /// consulted rather than implying it just looked.
    private static var cache: [String: (binding: IssuerBinding, expires: Date)] = [:]
    private static let answerTtl: TimeInterval = 900   // 15 min for a real answer
    private static let failureTtl: TimeInterval = 30   // a blip must clear on its own
    private static let lock = NSLock()

    /// The normative TXT name binding `clone` to `domain`. `nil` when either input is unusable, so the
    /// caller reports "could not check" instead of firing a nonsense query.
    static func txtName(clone: String, domain: String) -> String? {
        let addr = clone.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        let dom = domain.trimmingCharacters(in: .whitespacesAndNewlines)
            .lowercased()
            .trimmingCharacters(in: CharacterSet(charactersIn: "."))
        // Strict ASCII hex, not `isHexDigit`: that also accepts fullwidth forms, while Kotlin's
        // `Char.isDigit()` accepts non-ASCII digits — two ports loose in DIFFERENT directions on a name
        // the three legs must agree on byte for byte.
        guard addr.count == 42, addr.hasPrefix("0x"),
              addr.dropFirst(2).allSatisfy({ $0.isASCII && $0.isHexDigit }) else { return nil }
        guard dom.contains("."), !dom.contains("/"), !dom.contains(":"),
              !dom.contains(where: { $0.isWhitespace }) else { return nil }
        return "\(addr).\(label).\(dom)"
    }

    /// Which contract the chain says issued this credential, given the factory's `rootIssuer[R]` answer
    /// and the document's own `issuer.documentStore` claim.
    ///
    /// Pure, so the property that matters is testable without a network: a document whose
    /// `documentStore` points at some OTHER factory clone must never cause that clone's identity to be
    /// resolved. The chain's answer always wins; the document is a labelled last resort, reached only
    /// when the factory has no record of the root at all.
    static func chooseClone(rootIssuer: RoaxRpc.AddressRead, documentStore: String) -> IssuerCloneChoice {
        let claimed = documentStore.trimmingCharacters(in: .whitespacesAndNewlines)
        switch rootIssuer {
        case .value(let onchain):
            return IssuerCloneChoice(
                address: onchain,
                source: .rootIssuer,
                documentStoreDiffers: onchain.lowercased() != claimed.lowercased(),
                readFailed: false
            )
        case .noRecord:
            // The factory answered, and its answer is "no record of this root". The document's claim is
            // then the only thing available — used, but never presented as authoritative.
            return IssuerCloneChoice(address: claimed, source: .documentClaim, readFailed: false)
        case .failure:
            // We could not ask. NOT the same as "no record", and it must not become a licence to trust
            // the document's claim unchecked.
            return IssuerCloneChoice(address: claimed, source: .documentClaim, readFailed: true)
        }
    }

    /// Resolve the full chain for the credential whose Merkle root is `root`.
    ///
    /// `documentStore` is the document's CLAIM about its issuing contract and is never followed while the
    /// chain can answer: `rootIssuer[R]` is read first and, when it resolves, is the contract every link
    /// below is checked against.
    static func resolve(
        rpcUrl: String,
        factory: String,
        domainRegistry: String,
        documentStore: String,
        root: String,
        useCache: Bool = true
    ) async -> IssuerBinding {
        // The root is part of the key: two credentials can share a `documentStore` and still resolve to
        // different clones, so keying on the document's claim alone would serve one credential's answer
        // for another's.
        let key = [
            documentStore.lowercased(), root.lowercased(),
            domainRegistry.lowercased(), factory.lowercased(),
        ].joined(separator: "|")
        if useCache, let hit = cached(key) { return hit }

        var out = IssuerBinding()
        out.cloneAddress = documentStore.trimmingCharacters(in: .whitespacesAndNewlines)
        out.blockNumber = await RoaxRpc.blockNumber(rpcUrl: rpcUrl)

        // ---- link 0: WHICH contract, per the chain ---------------------------------------------
        //
        // Without a factory there is nothing to ask, and no link of the chain can be established — so
        // this is "could not read", never a fall-through to trusting the document.
        guard !factory.isEmpty else {
            out.state = .unavailable
            store(key, out)
            return out
        }
        let choice = chooseClone(
            rootIssuer: await RoaxRpc.rootIssuer(
                rpcUrl: rpcUrl, factory: factory, root: root, atBlock: out.blockNumber
            ),
            documentStore: documentStore
        )
        out.cloneAddress = choice.address
        out.cloneSource = choice.source
        out.documentStoreDiffers = choice.documentStoreDiffers
        if choice.readFailed || choice.address.isEmpty {
            out.state = .unavailable
            store(key, out)
            return out
        }
        let clone = choice.address

        // ---- link 1: factory provenance --------------------------------------------------------
        switch await RoaxRpc.isClone(rpcUrl: rpcUrl, factory: factory, candidate: clone, atBlock: out.blockNumber) {
        case .invalid:
            out.state = .notADogTagIssuer
            store(key, out)
            return out
        case .unknown:
            // The read did not resolve. NOT "not a DogTag issuer" — we do not know.
            out.state = .unavailable
            store(key, out)
            return out
        case .valid:
            break
        }

        // The authoritative issuer name, read once provenance holds.
        if case .value(let n) = await RoaxRpc.issuerOnchainName(rpcUrl: rpcUrl, clone: clone, atBlock: out.blockNumber) {
            out.onchainName = n
        }

        // ---- link 2: the on-chain domain claim -------------------------------------------------
        guard !domainRegistry.isEmpty else {
            out.state = .unavailable
            store(key, out)
            return out
        }
        switch await RoaxRpc.issuerClaimedDomain(
            rpcUrl: rpcUrl, domainRegistry: domainRegistry, clone: clone, atBlock: out.blockNumber
        ) {
        case .failure:
            out.state = .unavailable
            store(key, out)
            return out
        case .noContract:
            // An EMPTY eth_call result means no contract at that address — the registry is not deployed
            // for this config. That is "we could not check", never "this issuer claims no domain".
            out.state = .unavailable
            store(key, out)
            return out
        case .value(let d):
            let domain = d.trimmingCharacters(in: .whitespacesAndNewlines)
            if domain.isEmpty {
                // A real, ABI-encoded empty string: the issuer genuinely claims no domain.
                out.state = .noDomainClaimed
                store(key, out)
                return out
            }
            out.domain = domain
        }

        // ---- link 3: the DNS half ---------------------------------------------------------------
        out.checkedAt = Date()
        guard let name = txtName(clone: clone, domain: out.domain) else {
            out.state = .couldNotCheck
            store(key, out)
            return out
        }
        out.state = await resolveDns(name: name)
        store(key, out)
        return out
    }

    /// Resolve the TXT record over DoH. The classification itself is [`classifyDoh`], kept pure so the
    /// three-state rule is unit-testable without a network.
    private static func resolveDns(name: String) async -> IssuerBindingState {
        guard let encoded = name.addingPercentEncoding(
            withAllowedCharacters: CharacterSet.alphanumerics.union(CharacterSet(charactersIn: "-._~"))
        ) else {
            return .couldNotCheck
        }
        let url = "https://cloudflare-dns.com/dns-query?name=\(encoded)&type=TXT"
        let resp = await Http.getJSON(url, accept: "application/dns-json")
        guard resp.ok, let d = resp.body.data(using: .utf8),
              let o = (try? JSONSerialization.jsonObject(with: d)) as? [String: Any] else {
            return .couldNotCheck
        }
        return classifyDoh(o, queriedName: name)
    }

    /// Turn a DoH JSON answer into one of the three DNS states, branching on the `Status` RCODE so a
    /// resolver failure can never collapse into an absence. This is the mirror of `dogtag-dns-rs`'s
    /// `classify_txt` + `select_binding` and must stay behaviourally identical to them.
    static func classifyDoh(_ o: [String: Any], queriedName: String = "") -> IssuerBindingState {
        // An ABSENT Status is not "no error" — it means this is not a DoH JSON answer, and reading it as
        // NOERROR would turn a misconfiguration into a confident "absent".
        guard let status = o["Status"] as? Int else { return .couldNotCheck }
        switch status {
        case 0: break                      // NOERROR — inspect the answers
        case 3: return .notListed          // NXDOMAIN — a DEFINITIVE absence, not a failure
        default: return .couldNotCheck     // SERVFAIL(2) / REFUSED(5) / … — a non-answer
        }
        guard let answers = o["Answer"] as? [[String: Any]] else {
            return .notListed              // NOERROR with no Answer section (NODATA)
        }
        // COLLECT every TXT record before selecting one — a first-match-wins loop cannot implement the
        // rule below, because the single-record case must be accepted regardless of its echoed name.
        var records: [(name: String, value: String)] = []
        for a in answers {
            // type 16 == TXT; the Answer array may also carry CNAMEs from an alias chain.
            guard (a["type"] as? Int) == 16, let raw = a["data"] as? String else { continue }
            // DNS 0x20 means the echoed name's case is not guaranteed, so normalise it here once.
            let name = ((a["name"] as? String) ?? "")
                .trimmingCharacters(in: CharacterSet(charactersIn: "."))
                .lowercased()
            records.append((name: name, value: unquoteTxt(raw)))
        }
        guard let picked = selectBinding(records, queriedName: queriedName) else { return .notListed }
        return .verified(description: picked)
    }

    /// The TXT value published AT `queriedName`, if any. Mirror of `dogtag-dns-rs`'s `select_binding`.
    ///
    /// A SINGLE answer whose echoed name differs is still accepted, because that is what a CNAME chain
    /// looks like — the resolver answered the question it was asked. Two or more are not: there the
    /// echoed name is the only way to tell which record belongs to our query, and taking whichever
    /// happened to be first would let an unrelated record (an SPF line at a CNAME target, say) be
    /// displayed as this domain's description of the issuer.
    static func selectBinding(
        _ records: [(name: String, value: String)], queriedName: String
    ) -> String? {
        if records.count == 1 { return records[0].value }
        let want = queriedName
            .trimmingCharacters(in: CharacterSet(charactersIn: "."))
            .lowercased()
        return records.first(where: { $0.name == want })?.value
    }

    /// Join a DoH TXT value into the string the zone published. TXT RDATA is a sequence of
    /// character-strings each at most 255 octets, rendered as space-separated quoted chunks; RFC 1035
    /// says a consumer CONCATENATES them, so `"abc" "def"` is `abcdef`, not `abc def`. Getting this wrong
    /// would corrupt any description over 255 bytes.
    static func unquoteTxt(_ raw: String) -> String {
        let s = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        guard s.contains("\"") else { return s }
        var out = ""
        var inQuotes = false
        var escaped = false
        for ch in s {
            if escaped { out.append(ch); escaped = false; continue }
            if ch == "\\", inQuotes { escaped = true; continue }
            if ch == "\"" { inQuotes.toggle(); continue }
            if inQuotes { out.append(ch) }
            // whitespace between chunks is dropped, per concatenation semantics
        }
        return out
    }

    private static func cached(_ key: String) -> IssuerBinding? {
        lock.lock(); defer { lock.unlock() }
        guard let hit = cache[key], hit.expires > Date() else { return nil }
        return hit.binding
    }

    private static func store(_ key: String, _ b: IssuerBinding) {
        let ttl: TimeInterval
        switch b.state {
        case .couldNotCheck, .unavailable: ttl = failureTtl
        default: ttl = answerTtl
        }
        lock.lock(); defer { lock.unlock() }
        cache[key] = (b, Date().addingTimeInterval(ttl))
    }

    /// Drop every cached observation (used on an explicit refresh).
    static func clearCache() {
        lock.lock(); defer { lock.unlock() }
        cache.removeAll()
    }
}

// -------------------------------------------------------------------------------------------------
// Copy — the observation, never a verdict
// -------------------------------------------------------------------------------------------------

extension IssuerBinding {
    /// One factual line: what was looked at, and what was found. Deliberately contains no
    /// "VERIFICATION FAILED", "INVALID", "UNTRUSTED" or warning language — a missing DNS record means the
    /// domain owner has not published the binding, and says nothing about the credential, whose validity
    /// is separately and genuinely proven on-chain. Telling someone their valid credential failed would
    /// be worse than showing nothing.
    var line: String {
        let dom = domain.isEmpty ? "this domain" : domain
        switch state {
        case .verified:        return "This address is listed in \(dom)'s DNS records"
        case .notListed:       return "This address is not listed in \(dom)'s DNS records"
        case .notADogTagIssuer: return "This contract was not deployed by the DogTag factory"
        case .couldNotCheck:   return "We could not reach DNS to check this domain"
        case .noDomainClaimed: return "This issuer has published no domain on-chain"
        case .unavailable:     return "The on-chain domain claim could not be read"
        case .pending:         return "Checking this domain's DNS records…"
        }
    }

    /// The chain's write-once record names a different issuing contract than the document does.
    ///
    /// Stated as the observation it is, in the same register as the rest: no verdict about the
    /// credential, whose validity is proven on-chain separately. What it means concretely is that the
    /// identity shown above was resolved from the CHAIN's contract, not from the one the document names.
    var documentStoreLine: String? {
        guard documentStoreDiffers else { return nil }
        return "The chain records a different issuing contract than this document names"
    }

    /// The domain owner's own description, shown only for a verified binding — the whole reason the TXT
    /// value is free-form.
    var publishedDescription: String? {
        if case .verified(let d) = state {
            let t = d.trimmingCharacters(in: .whitespacesAndNewlines)
            return t.isEmpty ? nil : t
        }
        return nil
    }

    /// Which register the line is shown in. Only a verified binding is positive; only a definitive
    /// absence or a provenance failure is negative. Everything we simply do not know stays NEUTRAL — a
    /// resolver timeout is not evidence either way, so colouring it as a failure would be a lie of
    /// emphasis.
    enum Tone { case positive, negative, neutral, pending }

    var tone: Tone {
        switch state {
        case .verified: return .positive
        case .notListed, .notADogTagIssuer: return .negative
        case .pending: return .pending
        case .couldNotCheck, .noDomainClaimed, .unavailable: return .neutral
        }
    }

    /// Did this answer actually involve a DNS query? Only these three states reach link 3 — the resolver
    /// returns before it for a provenance failure, an unreadable claim, or no claim at all.
    ///
    /// Mirror of the TS `hasDnsHalf`; the three legs must agree or one surface claims a lookup another
    /// knows never happened.
    var hasDnsHalf: Bool {
        switch state {
        case .verified, .notListed, .couldNotCheck: return true
        case .notADogTagIssuer, .noDomainClaimed, .unavailable, .pending: return false
        }
    }

    /// Which block the chain half came from, and — ONLY when a DNS query really ran — when DNS was
    /// observed. `nil` when there is nothing honest to say.
    ///
    /// Saying "DNS checked just now" in a state that never queried DNS is precisely the fabrication the
    /// three-state design exists to prevent, so the DNS clause is gated on [`hasDnsHalf`] AND on having
    /// a real `checkedAt`. Answers are cached, keeping their ORIGINAL timestamp, so a stale one says
    /// "as recorded earlier" rather than claiming a fresh look.
    ///
    /// Mirror of the TS `bindingProvenanceLine`.
    func provenanceLine(now: Date = Date()) -> String? {
        var parts: [String] = []
        if let block = blockNumber { parts.append("chain read at block \(String(block))") }
        if hasDnsHalf, let seen = checkedAt {
            parts.append(
                now.timeIntervalSince(seen) < 60
                    ? "DNS checked just now (DNS has no history, so it cannot be re-checked for the past)"
                    : "DNS as recorded earlier (DNS has no history, so it cannot be re-checked for the past)"
            )
        }
        return parts.isEmpty ? nil : parts.joined(separator: " · ")
    }
}

// -------------------------------------------------------------------------------------------------
// The DID assertion — the OTHER half of issuer identity (audit-m9 recommendation 6)
// -------------------------------------------------------------------------------------------------

/// Comparing the DISPLAYED `issuer.domain` with the root-covered `data.issuer` DID.
///
/// This is NOT the same check as the DNS binding, and the brief requires both:
///  * the DNS binding proves the domain owner vouches for the contract address;
///  * this proves the document has not been RELABELLED since issuance.
///
/// Neither substitutes for the other. Mirror of `dogtag_standard::issuer_identity` — behaviour must stay
/// identical to the Rust, which the government API uses.
enum IssuerDidAssertion: Equatable {
    case match(domain: String)
    /// Positive evidence the issuer block was rewritten after issuance.
    case mismatch(displayed: String, rootCovered: String)
    /// No root-covered DID to compare against. NOT a pass — a skipped pillar contributing a pass is
    /// exactly how an unverified claim reaches a user looking verified.
    case notAssertable

    var isMismatch: Bool { if case .mismatch = self { return true }; return false }
}

enum IssuerIdentity {
    /// Extract the host from a `did:web:` DID. Path segments and a percent-encoded port are dropped;
    /// anything that is not a `did:web` yields nil.
    static func didWebHost(_ did: String) -> String? {
        let t = did.trimmingCharacters(in: .whitespacesAndNewlines)
        guard t.hasPrefix("did:web:") else { return nil }
        var host = String(t.dropFirst("did:web:".count))
        host = host.split(separator: ":", maxSplits: 1, omittingEmptySubsequences: false).first.map(String.init) ?? host
        for sep in ["%3A", "%3a"] {
            if let r = host.range(of: sep) { host = String(host[host.startIndex..<r.lowerBound]) }
        }
        host = host.trimmingCharacters(in: .whitespacesAndNewlines)
            .trimmingCharacters(in: CharacterSet(charactersIn: "."))
            .lowercased()
        return (host.isEmpty || !host.contains(".")) ? nil : host
    }

    /// The root-covered issuer DID from the `data.issuer` leaf, unpacking `<salt>:<tag>:<value>`.
    static func rootCoveredDid(_ doc: WrappedDoc?) -> String? {
        guard let raw = doc?.rootCoveredIssuerLeaf, !raw.isEmpty else { return nil }
        // A bare DID is used as-is; a packed leaf splits on the FIRST TWO colons only, because the
        // value itself contains colons (`did:web:...`).
        if raw.hasPrefix("did:") { return raw }
        let parts = raw.split(separator: ":", maxSplits: 2, omittingEmptySubsequences: false)
        return parts.count == 3 ? String(parts[2]) : raw
    }

    /// Compare, before displaying either value.
    static func assertDomain(_ doc: WrappedDoc?) -> IssuerDidAssertion {
        let displayed = (doc?.issuerDomain ?? "")
            .trimmingCharacters(in: .whitespacesAndNewlines)
            .trimmingCharacters(in: CharacterSet(charactersIn: "."))
            .lowercased()
        guard !displayed.isEmpty, let root = rootCoveredDid(doc).flatMap(didWebHost) else {
            return .notAssertable
        }
        return root == displayed ? .match(domain: root) : .mismatch(displayed: displayed, rootCovered: root)
    }
}
