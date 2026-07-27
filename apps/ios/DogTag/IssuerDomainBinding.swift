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

/// A resolved binding plus the provenance needed to be honest about it.
struct IssuerBinding: Equatable {
    var state: IssuerBindingState = .pending
    /// The ON-CHAIN claimed domain that was queried.
    var domain: String = ""
    /// The clone's on-chain `name()` — the only authoritative issuer name.
    var onchainName: String = ""
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
        guard addr.count == 42, addr.hasPrefix("0x"),
              addr.dropFirst(2).allSatisfy({ $0.isHexDigit }) else { return nil }
        guard dom.contains("."), !dom.contains("/"), !dom.contains(":"),
              !dom.contains(where: { $0.isWhitespace }) else { return nil }
        return "\(addr).\(label).\(dom)"
    }

    /// Resolve the full chain. `clone` is the issuing contract; prefer the address the CHAIN resolved for
    /// the credential's root over the document's `issuer.documentStore` where available.
    static func resolve(
        rpcUrl: String,
        factory: String,
        domainRegistry: String,
        clone: String,
        useCache: Bool = true
    ) async -> IssuerBinding {
        let key = "\(clone.lowercased())|\(domainRegistry.lowercased())|\(factory.lowercased())"
        if useCache, let hit = cached(key) { return hit }

        var out = IssuerBinding()
        out.blockNumber = await RoaxRpc.blockNumber(rpcUrl: rpcUrl)

        // ---- link 1: factory provenance --------------------------------------------------------
        guard !factory.isEmpty else {
            out.state = .unavailable
            store(key, out)
            return out
        }
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
        return classifyDoh(o)
    }

    /// Turn a DoH JSON answer into one of the three DNS states, branching on the `Status` RCODE so a
    /// resolver failure can never collapse into an absence. This is the mirror of `dogtag-dns-rs`'s
    /// `classify_txt` and must stay behaviourally identical to it.
    static func classifyDoh(_ o: [String: Any]) -> IssuerBindingState {
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
        for a in answers {
            // type 16 == TXT; the Answer array may also carry CNAMEs from an alias chain.
            guard (a["type"] as? Int) == 16, let raw = a["data"] as? String else { continue }
            return .verified(description: unquoteTxt(raw))
        }
        return .notListed
    }

    /// Join a DoH TXT value into the string the zone published. TXT RDATA is a sequence of
    /// character-strings each at most 255 octets, rendered as space-separated quoted chunks; RFC 1035
    /// says a consumer CONCATENATES them, so `"abc" "def"` is `abcdef`, not `abc def`. Getting this wrong
    /// would corrupt any description over 255 bytes.
    static func unquoteTxt(_ raw: String) -> String {
        let s = raw.trimmingCharacters(in: .whitespaces)
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
}
