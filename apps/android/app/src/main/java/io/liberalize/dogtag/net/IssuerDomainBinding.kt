package io.liberalize.dogtag.net

import io.liberalize.dogtag.data.WrappedDoc
import org.json.JSONObject
import java.net.URLEncoder

/**
 * The issuer↔domain binding, resolved on the phone. Mirror of `apps/ios/DogTag/IssuerDomainBinding.swift`
 * and of the Rust `dogtag-dns-rs` classification rule; the three must stay behaviourally identical.
 *
 * # The three-link chain
 *
 * All three links are required, and all three are checked HERE at display time rather than inherited
 * from anything a server or a document says:
 *
 *  1. **Factory provenance** — `DogTagIssuerFactory.isClone(clone)`. Without it the rest is worthless:
 *     anyone can deploy their own contract, claim a domain for it and publish a matching TXT record.
 *     DNS would agree, the registry would agree, and none of it would mean anything, because that
 *     contract never passed through the KYC-gated `createIssuer`.
 *  2. **The on-chain domain claim** — `IssuerDomainRegistry.domainOf(clone)`. Read from the CHAIN, never
 *     from the credential: the document's `issuer` block is outside the Merkle root, so a relabelled
 *     document verifies. Taking the domain from the chain removes that attack entirely.
 *  3. **The DNS half** — that domain publishes
 *     `<clone-address-lowercase>._dogtag.<domain> TXT "<free-form description>"`.
 *
 * # Where the DNS lookup happens, and what it costs
 *
 * The phone resolves DNS-over-HTTPS DIRECTLY, with no DogTag server in the loop. Routing it through our
 * own API would tell US which credential the holder is looking at and when — strictly worse for a
 * privacy-sensitive wallet than telling a public resolver a domain name. It also matches the existing
 * precedent in [DnsVerify], which already resolves DoH directly for the groomer binding.
 *
 * The privacy cost, stated plainly: **Cloudflare learns the issuer's domain and this device's IP at the
 * moment a credential is viewed.** It does not learn the credential, the holder, or the dogTagId.
 *
 * # No synthesized states
 *
 * Every state is the outcome of a real read, or of a real read that really failed. Nothing is defaulted,
 * and a surface that renders before resolution completes shows [IssuerBindingState.Pending] rather than
 * guessing — flashing "no domain claimed" and then flipping would show a state never observed.
 */
sealed class IssuerBindingState {
    /** The domain publishes a record for this contract. [description] is the domain owner's own words. */
    data class Verified(val description: String) : IssuerBindingState()

    /**
     * Link 1 failed: this contract did not come from the DogTag factory. Categorically STRONGER than a
     * missing DNS record and must never be shown as one.
     */
    object NotADogTagIssuer : IssuerBindingState()

    /** Links 1 and 2 hold; the domain publishes no record for this address. A NORMAL state. */
    object NotListed : IssuerBindingState()

    /** A real DNS attempt failed. Evidence of nothing. */
    object CouldNotCheck : IssuerBindingState()

    /** Link 1 holds; this issuer has claimed no domain on-chain. The normal day-one state. */
    object NoDomainClaimed : IssuerBindingState()

    /**
     * A provider-DIRECTORY listing carried no domain. No chain read happened at all, so this must
     * never borrow [NoDomainClaimed]'s on-chain wording, which asserts a read that did occur.
     */
    object NoDomainListed : IssuerBindingState()

    /** A prerequisite could not be read at all (nothing configured, or the read failed). */
    object Unavailable : IssuerBindingState()

    /** In flight. Never a resting state. */
    object Pending : IssuerBindingState()
}

/** Which register a state is shown in. */
enum class BindingTone { Positive, Negative, Neutral, Pending }

/**
 * Where the contract a binding was resolved against came from.
 *
 * Not cosmetic: the two carry different authority, and the difference is the whole of the sharper
 * relabelling attack. [RootIssuer] is the factory's write-once record of which clone issued THIS root.
 * [DocumentClaim] is the credential's own `issuer.documentStore`, which sits outside the Merkle root and
 * can be pointed at any address — including another authority's genuine, factory-deployed clone.
 */
enum class IssuerCloneSource {
    /** The factory's write-once `rootIssuer[R]`. Authoritative. */
    RootIssuer,

    /**
     * The document's own claim, reached only because the factory has no record of this root. Never
     * authoritative, and always labelled as the fallback it is.
     */
    DocumentClaim,
}

/**
 * The pure decision "which contract does this binding describe", separated from the I/O so it is
 * testable without a network. See [IssuerBindingResolver.chooseClone].
 */
data class IssuerCloneChoice(
    /** The contract every link of the chain is then resolved against. */
    val address: String = "",
    val source: IssuerCloneSource = IssuerCloneSource.DocumentClaim,
    /** The chain names a DIFFERENT issuing contract than the document does. Reported, never followed. */
    val documentStoreDiffers: Boolean = false,
    /**
     * The `rootIssuer` read did not resolve at all. The document's claim is then unchecked, so the caller
     * must report "could not read" rather than proceeding on it.
     */
    val readFailed: Boolean = false,
)

/** A resolved binding plus the provenance needed to be honest about it. */
data class IssuerBinding(
    val state: IssuerBindingState = IssuerBindingState.Pending,
    /** The ON-CHAIN claimed domain that was queried. */
    val domain: String = "",
    /** The clone's on-chain `name()` — the only authoritative issuer name. */
    val onchainName: String = "",
    /** The contract this binding actually describes, and where that address came from. */
    val cloneAddress: String = "",
    val cloneSource: IssuerCloneSource = IssuerCloneSource.DocumentClaim,
    /**
     * The chain's write-once `rootIssuer[R]` names a different contract than the document's
     * `issuer.documentStore`. Positive evidence the document's claim was swapped.
     */
    val documentStoreDiffers: Boolean = false,
    /**
     * The block every on-chain read was pinned to. Null == the head could not be read, so this answer is
     * not reproducible and must not imply a block.
     */
    val blockNumber: Long? = null,
    /** Epoch millis the DNS half was observed. DNS has no history, so this is its only timestamp. */
    val checkedAt: Long? = null,
) {
    val isVerified: Boolean get() = state is IssuerBindingState.Verified

    /**
     * One factual line: what was looked at, and what was found. Deliberately contains no "VERIFICATION
     * FAILED", "INVALID", "UNTRUSTED" or warning language — a missing DNS record means the domain owner
     * has not published the binding, and says nothing about the credential, whose validity is separately
     * and genuinely proven on-chain. Telling a holder their valid credential failed would be worse than
     * showing nothing.
     */
    val line: String
        get() {
            val dom = domain.ifBlank { "this domain" }
            return when (state) {
                is IssuerBindingState.Verified -> "This address is listed in $dom's DNS records"
                IssuerBindingState.NotListed -> "This address is not listed in $dom's DNS records"
                // Deliberately says nothing about DNS: DNS was never the question here.
                IssuerBindingState.NotADogTagIssuer -> "This contract was not deployed by the DogTag factory"
                IssuerBindingState.CouldNotCheck -> "We could not reach DNS to check this domain"
                IssuerBindingState.NoDomainClaimed -> "This issuer has published no domain on-chain"
                IssuerBindingState.NoDomainListed -> "No domain listed for this provider"
                IssuerBindingState.Unavailable -> "The on-chain domain claim could not be read"
                IssuerBindingState.Pending -> "Checking this domain's DNS records…"
            }
        }

    /**
     * The chain's write-once record names a different issuing contract than the document does.
     *
     * Stated as the observation it is, in the same register as the rest: no verdict about the
     * credential, whose validity is proven on-chain separately. What it means concretely is that the
     * identity shown above was resolved from the CHAIN's contract, not from the one the document names.
     */
    val documentStoreLine: String?
        get() = if (documentStoreDiffers) {
            "The chain records a different issuing contract than this document names"
        } else {
            null
        }

    /** The domain owner's own description — the whole reason the TXT value is free-form. */
    val publishedDescription: String?
        get() = (state as? IssuerBindingState.Verified)?.description?.trim()?.ifBlank { null }

    /**
     * Only a verified binding is positive; only a definitive absence or a provenance failure is negative.
     * Everything we simply do not know stays NEUTRAL — a resolver timeout is not evidence either way, so
     * colouring it as a failure would be a lie of emphasis.
     */
    val tone: BindingTone
        get() = when (state) {
            is IssuerBindingState.Verified -> BindingTone.Positive
            IssuerBindingState.NotListed, IssuerBindingState.NotADogTagIssuer -> BindingTone.Negative
            IssuerBindingState.Pending -> BindingTone.Pending
            else -> BindingTone.Neutral
        }

    /**
     * Did this answer actually involve a DNS query? Only these three states reach link 3 — the resolver
     * returns before it for a provenance failure, an unreadable claim, or no claim at all.
     *
     * Mirror of the TS `hasDnsHalf`; the three legs must agree or one surface claims a lookup another
     * knows never happened.
     */
    val hasDnsHalf: Boolean
        get() = when (state) {
            is IssuerBindingState.Verified,
            IssuerBindingState.NotListed,
            IssuerBindingState.CouldNotCheck,
            -> true
            else -> false
        }

    /**
     * Which block the chain half came from, and — ONLY when a DNS query really ran — when DNS was
     * observed. Null when there is nothing honest to say.
     *
     * Saying "DNS checked just now" in a state that never queried DNS is precisely the fabrication the
     * three-state design exists to prevent, so the DNS clause is gated on [hasDnsHalf] AND on having a
     * real [checkedAt]. Answers are cached, keeping their ORIGINAL timestamp, so a stale one says "as
     * recorded earlier" rather than claiming a fresh look.
     *
     * Mirror of the TS `bindingProvenanceLine` and of the Swift `provenanceLine`.
     */
    fun provenanceLine(now: Long = System.currentTimeMillis()): String? {
        val parts = ArrayList<String>(2)
        blockNumber?.let { parts.add("chain read at block $it") }
        val seen = checkedAt
        if (hasDnsHalf && seen != null) {
            parts.add(
                if (now - seen < 60_000L) {
                    "DNS checked just now (DNS has no history, so it cannot be re-checked for the past)"
                } else {
                    "DNS as recorded earlier (DNS has no history, so it cannot be re-checked for the past)"
                },
            )
        }
        return if (parts.isEmpty()) null else parts.joinToString(" · ")
    }
}

object IssuerBindingResolver {
    /**
     * The underscore label that namespaces every DogTag binding record. An underscore label cannot
     * collide with a real hostname, so the binding can never shadow something the domain owner serves.
     * Address-label-first mirrors DKIM's `<selector>._domainkey.<domain>`.
     */
    const val LABEL = "_dogtag"

    private const val ANSWER_TTL_MS = 900_000L // 15 min for a real answer
    private const val FAILURE_TTL_MS = 30_000L // a blip must clear on its own

    private data class Entry(val binding: IssuerBinding, val expiresAt: Long)

    private val cache = HashMap<String, Entry>()
    private val lock = Any()

    /**
     * The normative TXT name binding [clone] to [domain]. Null when either input is unusable, so the
     * caller reports "could not check" instead of firing a nonsense query.
     */
    fun txtName(clone: String, domain: String): String? {
        val addr = clone.trim().lowercase()
        val dom = domain.trim().lowercase().trim('.')
        if (addr.length != 42 || !addr.startsWith("0x")) return null
        // Strict ASCII hex: `Char.isDigit()` is Unicode-aware and would accept e.g. an Arabic-Indic
        // digit, which the Swift and Rust legs do not.
        if (!addr.drop(2).all { it in '0'..'9' || it in 'a'..'f' }) return null
        if (!dom.contains('.') || dom.contains('/') || dom.contains(':')) return null
        if (dom.any { it.isWhitespace() }) return null
        return "$addr.$LABEL.$dom"
    }

    /**
     * Which contract the chain says issued this credential, given the factory's `rootIssuer[R]` answer
     * and the document's own `issuer.documentStore` claim.
     *
     * Pure, so the property that matters is testable without a network: a document whose `documentStore`
     * points at some OTHER factory clone must never cause that clone's identity to be resolved. The
     * chain's answer always wins; the document is a labelled last resort, reached only when the factory
     * has no record of the root at all.
     *
     * Mirror of the Swift `IssuerBindingResolver.chooseClone`.
     */
    fun chooseClone(rootIssuer: RoaxRpc.AddressRead, documentStore: String): IssuerCloneChoice {
        val claimed = documentStore.trim()
        return when (rootIssuer) {
            is RoaxRpc.AddressRead.Value -> IssuerCloneChoice(
                address = rootIssuer.address,
                source = IssuerCloneSource.RootIssuer,
                documentStoreDiffers = !rootIssuer.address.equals(claimed, ignoreCase = true),
            )
            // The factory answered, and its answer is "no record of this root". The document's claim is
            // then the only thing available — used, but never presented as authoritative.
            is RoaxRpc.AddressRead.NoRecord ->
                IssuerCloneChoice(address = claimed, source = IssuerCloneSource.DocumentClaim)
            // We could not ask. NOT the same as "no record", and it must not become a licence to trust
            // the document's claim unchecked.
            is RoaxRpc.AddressRead.Failure ->
                IssuerCloneChoice(address = claimed, source = IssuerCloneSource.DocumentClaim, readFailed = true)
        }
    }

    /**
     * Identity of one cached chain/DNS observation.
     *
     * The endpoint is load-bearing: changing peers must not replay the former peer's answer. Keep
     * the normalized endpoint's path and query case intact because both can carry case-sensitive
     * routing or credentials; lowercasing the whole URL would collapse distinct peers.
     */
    internal fun cacheKey(
        rpcUrl: String,
        expectedChainId: Long,
        factory: String,
        domainRegistry: String,
        documentStore: String,
        root: String,
    ): String = listOf(
        RoaxRpc.normalizeRpcUrl(rpcUrl) ?: rpcUrl.trim(),
        expectedChainId.toString(),
        documentStore.lowercase(), root.lowercase(),
        domainRegistry.lowercase(), factory.lowercase(),
    ).joinToString("|")

    /**
     * Resolve the full chain for the credential whose Merkle root is [root].
     *
     * [documentStore] is the document's CLAIM about its issuing contract and is never followed while the
     * chain can answer: `rootIssuer[R]` is read first and, when it resolves, is the contract every link
     * below is checked against.
     */
    suspend fun resolve(
        rpcUrl: String,
        expectedChainId: Long,
        factory: String,
        domainRegistry: String,
        documentStore: String,
        root: String,
        useCache: Boolean = true,
    ): IssuerBinding {
        // The root is part of the key: two credentials can share a `documentStore` and still resolve to
        // different clones, so keying on the document's claim alone would serve one credential's answer
        // for another's.
        val key = cacheKey(
            rpcUrl, expectedChainId, factory, domainRegistry, documentStore, root,
        )
        if (useCache) cached(key)?.let { return it }

        val block = RoaxRpc.blockNumber(rpcUrl, expectedChainId)
        val claimed = documentStore.trim()

        // ---- link 0: WHICH contract, per the chain ---------------------------------------------
        //
        // Without a factory there is nothing to ask, and no link of the chain can be established — so
        // this is "could not read", never a fall-through to trusting the document.
        if (factory.isBlank()) {
            return store(
                key,
                IssuerBinding(IssuerBindingState.Unavailable, cloneAddress = claimed, blockNumber = block),
            )
        }
        val choice = chooseClone(
            RoaxRpc.rootIssuer(rpcUrl, expectedChainId, factory, root, block),
            documentStore,
        )
        val chain = { s: IssuerBindingState, name: String, dom: String, seen: Long? ->
            IssuerBinding(
                state = s,
                domain = dom,
                onchainName = name,
                cloneAddress = choice.address,
                cloneSource = choice.source,
                documentStoreDiffers = choice.documentStoreDiffers,
                blockNumber = block,
                checkedAt = seen,
            )
        }
        if (choice.readFailed || choice.address.isBlank()) {
            return store(key, chain(IssuerBindingState.Unavailable, "", "", null))
        }
        val clone = choice.address

        // ---- link 1: factory provenance --------------------------------------------------------
        when (RoaxRpc.isClone(rpcUrl, expectedChainId, factory, clone, block)) {
            is RoaxRpc.Result.Invalid ->
                return store(key, chain(IssuerBindingState.NotADogTagIssuer, "", "", null))
            is RoaxRpc.Result.Unknown ->
                // The read did not resolve. NOT "not a DogTag issuer" — we do not know.
                return store(key, chain(IssuerBindingState.Unavailable, "", "", null))
            is RoaxRpc.Result.Valid -> Unit
        }

        // The authoritative issuer name, read once provenance holds.
        val onchainName = (RoaxRpc.issuerOnchainName(
            rpcUrl, expectedChainId, clone, block,
        ) as? RoaxRpc.StringRead.Value)
            ?.value.orEmpty()

        // ---- link 2: the on-chain domain claim -------------------------------------------------
        if (domainRegistry.isBlank()) {
            return store(key, chain(IssuerBindingState.Unavailable, onchainName, "", null))
        }
        val domain = when (
            val r = RoaxRpc.issuerClaimedDomain(
                rpcUrl, expectedChainId, domainRegistry, clone, block,
            )
        ) {
            is RoaxRpc.StringRead.Failure ->
                return store(key, chain(IssuerBindingState.Unavailable, onchainName, "", null))
            // An EMPTY eth_call result means no contract at that address — the registry is not deployed
            // for this config. That is "we could not check", never "this issuer claims no domain".
            is RoaxRpc.StringRead.NoContract ->
                return store(key, chain(IssuerBindingState.Unavailable, onchainName, "", null))
            is RoaxRpc.StringRead.Value -> r.value.trim()
        }
        if (domain.isEmpty()) {
            // A real, ABI-encoded empty string: the issuer genuinely claims no domain.
            return store(key, chain(IssuerBindingState.NoDomainClaimed, onchainName, "", null))
        }

        // ---- link 3: the DNS half ---------------------------------------------------------------
        val observedAt = System.currentTimeMillis()
        val name = txtName(clone, domain)
            ?: return store(key, chain(IssuerBindingState.CouldNotCheck, onchainName, domain, observedAt))
        return store(key, chain(resolveDns(name), onchainName, domain, observedAt))
    }

    private suspend fun resolveDns(name: String): IssuerBindingState {
        return try {
            val url = "https://cloudflare-dns.com/dns-query?name=${URLEncoder.encode(name, "UTF-8")}&type=TXT"
            val resp = Http.getJsonAccept(url, accept = "application/dns-json")
            if (!resp.ok) return IssuerBindingState.CouldNotCheck
            classifyDoh(JSONObject(resp.body), queriedName = name)
        } catch (e: Exception) {
            IssuerBindingState.CouldNotCheck
        }
    }

    /**
     * Turn a DoH JSON answer into one of the three DNS states, branching on the `Status` RCODE so a
     * resolver failure can never collapse into an absence. Pure, so the rule is unit-testable without a
     * network; mirror of `dogtag_dns_rs::classify_txt` + `select_binding`.
     */
    fun classifyDoh(o: JSONObject, queriedName: String = ""): IssuerBindingState {
        // An ABSENT (or non-numeric) Status is not "no error" — it means this is not a DoH JSON answer,
        // and reading it as NOERROR would turn a misconfiguration into a confident "absent". A DNS RCODE
        // is 0..15, so -1 is an unambiguous "no usable Status" sentinel and needs no `has()` probe (which
        // the JVM unit-test stub of org.json does not implement).
        when (o.optInt("Status", -1)) {
            0 -> Unit                                   // NOERROR — inspect the answers
            3 -> return IssuerBindingState.NotListed     // NXDOMAIN — a DEFINITIVE absence
            else -> return IssuerBindingState.CouldNotCheck // SERVFAIL(2) / REFUSED(5) / absent / … — a non-answer
        }
        val answers = o.optJSONArray("Answer")
            ?: return IssuerBindingState.NotListed       // NOERROR with no Answer section (NODATA)
        // COLLECT every TXT record before selecting one — a first-match-wins loop cannot implement the
        // rule below, because the single-record case must be accepted regardless of its echoed name.
        val records = ArrayList<Pair<String, String>>()
        for (i in 0 until answers.length()) {
            val a = answers.optJSONObject(i) ?: continue
            // type 16 == TXT; the Answer array may also carry CNAMEs from an alias chain.
            if (a.optInt("type", -1) != 16) continue
            val raw = a.optString("data", "")
            if (raw.isEmpty()) continue
            // DNS 0x20 means the echoed name's case is not guaranteed, so normalise it here once.
            val name = a.optString("name", "").trim('.').lowercase()
            records.add(name to unquoteTxt(raw))
        }
        val picked = selectBinding(records, queriedName) ?: return IssuerBindingState.NotListed
        return IssuerBindingState.Verified(picked)
    }

    /**
     * The TXT value published AT [queriedName], if any. Mirror of `dogtag_dns_rs::select_binding`.
     *
     * A SINGLE answer whose echoed name differs is still accepted, because that is what a CNAME chain
     * looks like — the resolver answered the question it was asked. Two or more are not: there the echoed
     * name is the only way to tell which record belongs to our query, and taking whichever happened to be
     * first would let an unrelated record (an SPF line at a CNAME target, say) be displayed as this
     * domain's description of the issuer.
     */
    fun selectBinding(records: List<Pair<String, String>>, queriedName: String): String? {
        if (records.size == 1) return records[0].second
        val want = queriedName.trim('.').lowercase()
        return records.firstOrNull { it.first == want }?.second
    }

    /**
     * Join a DoH TXT value into the string the zone published. TXT RDATA is a sequence of
     * character-strings each at most 255 octets, rendered as space-separated quoted chunks; RFC 1035 says
     * a consumer CONCATENATES them, so `"abc" "def"` is `abcdef`, not `abc def`. Getting this wrong would
     * corrupt any description over 255 bytes.
     */
    fun unquoteTxt(raw: String): String {
        val s = raw.trim()
        if (!s.contains('"')) return s
        val out = StringBuilder()
        var inQuotes = false
        var escaped = false
        for (ch in s) {
            if (escaped) { out.append(ch); escaped = false; continue }
            when {
                ch == '\\' && inQuotes -> escaped = true
                ch == '"' -> inQuotes = !inQuotes
                inQuotes -> out.append(ch)
                // whitespace between chunks is dropped, per concatenation semantics
            }
        }
        return out.toString()
    }

    private fun cached(key: String): IssuerBinding? = synchronized(lock) {
        val e = cache[key] ?: return null
        if (e.expiresAt > System.currentTimeMillis()) e.binding else null
    }

    private fun store(key: String, b: IssuerBinding): IssuerBinding {
        val ttl = when (b.state) {
            IssuerBindingState.CouldNotCheck, IssuerBindingState.Unavailable -> FAILURE_TTL_MS
            else -> ANSWER_TTL_MS
        }
        synchronized(lock) { cache[key] = Entry(b, System.currentTimeMillis() + ttl) }
        return b
    }

    /** Drop every cached observation (used on an explicit refresh). */
    fun clearCache() = synchronized(lock) { cache.clear() }
}

// -------------------------------------------------------------------------------------------------
// The DID assertion — the OTHER half of issuer identity (audit-m9 recommendation 6)
// -------------------------------------------------------------------------------------------------

/**
 * Comparing the DISPLAYED `issuer.domain` with the root-covered `data.issuer` DID.
 *
 * This is NOT the same check as the DNS binding, and the brief requires both:
 *  * the DNS binding proves the domain owner vouches for the contract address;
 *  * this proves the document has not been RELABELLED since issuance.
 *
 * Neither substitutes for the other. Mirror of `dogtag_standard::issuer_identity` and of the Swift
 * `IssuerIdentity` — behaviour must stay identical across all three.
 */
sealed class IssuerDidAssertion {
    data class Match(val domain: String) : IssuerDidAssertion()

    /** Positive evidence the issuer block was rewritten after issuance. */
    data class Mismatch(val displayed: String, val rootCovered: String) : IssuerDidAssertion()

    /**
     * No root-covered DID to compare against. NOT a pass — a skipped pillar contributing a pass is
     * exactly how an unverified claim reaches a user looking verified.
     */
    object NotAssertable : IssuerDidAssertion()

    val isMismatch: Boolean get() = this is Mismatch
}

object IssuerIdentity {
    /**
     * Extract the host from a `did:web:` DID. Path segments and a percent-encoded port are dropped;
     * anything that is not a `did:web` yields null.
     */
    fun didWebHost(did: String): String? {
        val t = did.trim()
        if (!t.startsWith("did:web:")) return null
        var host = t.removePrefix("did:web:").substringBefore(':')
        host = host.substringBefore("%3A").substringBefore("%3a")
        host = host.trim().trim('.').lowercase()
        return if (host.isEmpty() || !host.contains('.')) null else host
    }

    /** The root-covered issuer DID from the `data.issuer` leaf, unpacking `<salt>:<tag>:<value>`. */
    fun rootCoveredDid(doc: WrappedDoc?): String? {
        val raw = doc?.rootCoveredIssuerLeaf?.takeIf { it.isNotBlank() } ?: return null
        // A bare DID is used as-is; a packed leaf splits on the FIRST TWO colons only, because the value
        // itself contains colons (`did:web:...`).
        if (raw.startsWith("did:")) return raw
        val parts = raw.split(":", limit = 3)
        return if (parts.size == 3) parts[2] else raw
    }

    /** Compare, before displaying either value. */
    fun assertDomain(doc: WrappedDoc?): IssuerDidAssertion {
        val displayed = (doc?.issuerDomain ?: "").trim().trim('.').lowercase()
        if (displayed.isEmpty()) return IssuerDidAssertion.NotAssertable
        val root = rootCoveredDid(doc)?.let { didWebHost(it) } ?: return IssuerDidAssertion.NotAssertable
        return if (root == displayed) IssuerDidAssertion.Match(root)
        else IssuerDidAssertion.Mismatch(displayed, root)
    }
}
