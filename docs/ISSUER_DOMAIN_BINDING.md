# The issuer↔domain binding

**Status:** normative.
This document defines the DNS record convention, the verification chain, and the display rules.
Implementations that disagree with this document are wrong.

**A successor to the on-chain half is DEPLOYED AND UNWIRED.**
Registry-plan slice S-9 adds `contracts/src/ServiceDomainResolver.sol`, which supersedes
`IssuerDomainRegistry` — see [SERVICE_DOMAIN_RESOLVER.md](./SERVICE_DOMAIN_RESOLVER.md).
Cutover step C-7 deployed it on ROAX on 2026-08-01 (`_c7_typed_resolvers` in
`contracts/deployments/roax.json`), but no registrar has approved it and no service has selected it,
so it resolves nothing and no consumer reads it; client repointing is C-9/C-10 and has not happened.
Nothing in this document changes because of it today: the deployed `IssuerDomainRegistry` is still the
wired contract, the record convention and the six display states are unchanged, and every consumer still
reads the address it always did.
Two things below become specific to the superseded contract once the cutover happens.
"Who may write a binding" describes its three tiers, which the successor replaces with the authority
core's cleared-standing-AND-confirmed-owner predicate.
And `noDomainClaimed` will be reachable only from a real `NO_DOMAIN` disposition rather than from any
empty string — the successor distinguishes "nobody has written anything", "there is deliberately no
domain" and "a claim was withdrawn", which are one state here, so the two new absences need their own
copy rather than borrowing that sentence.

## What problem this solves

An audit demonstrated live that the issuer a user sees is attacker-controlled.
`check_integrity` hashes only `doc.data` (flattened) plus `privacy.obfuscated`, so the wrapped document's top-level `issuer` block — `name`, `domain`, and `documentStore` — sits **outside the Merkle root**.
Taking a genuine credential, changing nothing in `data`, and relabelling the issuer to "Ministry of Health of Singapore / moh.gov.sg" still returned `verdict: true`.

Validity and revocation were never the problem: those are on-chain, and the organisation was KYC'd and whitelisted through the factory.
What was missing is any evidence that the organisation named on screen is the one that controls the domain it claims.

## What the binding proves, and what it does not

A verified binding proves exactly one thing: **the owner of that DNS zone published a record naming this contract address.**

It does **not** establish that the named organisation is who it says it is.
That comes from KYC at onboarding, which is a separate and independent root of trust.
Copy on every surface must state the observation and stop there — see [Display rules](#display-rules).

The binding is **bidirectional**, which is the point: the chain asserts "my domain is D" and D's zone asserts "contract C is mine".
Neither half alone is sufficient, so forging a displayed issuer identity requires **both** a key authorized on-chain **and** control of the DNS zone.

## The record convention

```
<clone-address-lowercase>._dogtag.<domain>.   IN TXT   "<free-form description>"
```

Concretely, for a clone at `0xCLONE…` claiming `moh.gov.sg` (illustrative - no clone exists on the launch set yet, since `providerCount` is 0):

```
0x<clone address, lowercased>._dogtag.moh.gov.sg. IN TXT "Travel clearance issuance"
```

**The address is in the NAME; the VALUE is free-form.**
That split is the design: the domain owner chooses what the record says about the address, so the value cannot also carry the address.
Presence of a TXT record at that exact name **is** the assertion — a verifier never pattern-matches the value, it only reports it.

### Why `_dogtag`

An underscore-prefixed label cannot appear in a legal hostname, so the binding can never shadow something the domain owner actually serves.
Putting the selector-like address label **first** and the underscore label second follows DKIM's `<selector>._domainkey.<domain>`, the closest established precedent for "one machine-read TXT record per key".
A `0x`-prefixed 20-byte address is 42 characters, comfortably inside the 63-octet DNS label limit.

### Casing

Addresses are frequently written EIP-55 checksummed (mixed case), so the query name is **always lowercased** before the lookup.
DNS is case-insensitive, and a resolver may additionally randomise the case of the name it echoes back (DNS 0x20), so every name comparison is case-insensitive too.

### Relationship to the apex `dogtag-verify=` record

There are **two** DogTag TXT conventions and they are deliberately different records with different shapes:

| Convention | Shape | Used by |
|---|---|---|
| **Apex, value-encoded** | `<domain> IN TXT "dogtag-verify=<address>"` | the admin onboarding legitimacy gate (`stacks/admin/api/src/dns.rs`) and the phone's pre-disclosure groomer binding (`DnsVerify.kt`, `ScanScreen.swift`) |
| **Issuer↔domain binding** | `<address>._dogtag.<domain> IN TXT "<description>"` | this document |

The apex convention is load-bearing for those two gates and is **not** changed.
The binding needs the value free for the domain owner's description, which forces the address into the name — the two shapes are incompatible, so they are separate records.

## The verification chain — three links, all required

```
   ┌───────────────────────────────────────────────────────────────────────────┐
   │ 1. FACTORY PROVENANCE   factory.isClone(clone) == true                    │
   │    the contract came from the DogTag factory, i.e. through KYC-gated      │
   │    createIssuer (onlyOwner, the protocol multisig)                        │
   └───────────────────────────────────────────────────────────────────────────┘
                                      ↓
   ┌───────────────────────────────────────────────────────────────────────────┐
   │ 2. THE ON-CHAIN CLAIM   IssuerDomainRegistry.domainOf(clone) == D         │
   │    read from the CHAIN, never from the credential document                │
   └───────────────────────────────────────────────────────────────────────────┘
                                      ↓
   ┌───────────────────────────────────────────────────────────────────────────┐
   │ 3. THE DNS HALF   <clone>._dogtag.D publishes a TXT record                │
   └───────────────────────────────────────────────────────────────────────────┘
```

**Link 1 is not optional and not redundant.**
Without it the rest is worthless: anyone can deploy their own contract, register a domain for it, publish a matching TXT record, and present as verified.
DNS would agree, the registry would agree, and none of it would mean anything, because that contract never passed through the KYC-gated `createIssuer`.
Factory provenance is what ties the binding back to the whitelisting that gives it value.

It is enforced in **both** places:

- `IssuerDomainRegistry` refuses a write for an address that is not a factory clone, so a bad binding cannot be **stored** at all.
- Every verifier re-checks provenance at verification time. A stored binding is a **claim**, and an app must not inherit trust it did not verify itself — the registry could be swapped, or a future one could be laxer.

**Link 2 must come from the chain.**
Reading the domain from the document would reintroduce the original attack: the `issuer` block is not root-covered, so a relabelled document could move it.
A relabelled document cannot move a value it does not carry.

## Which contract issued a credential

`issuer.documentStore` in the document is only the document's **claim**, and pointing it at a contract the attacker controls is the sharper form of the relabelling attack.
The authoritative answer is the factory's write-once `rootIssuer[R]`, which names the clone that issued **that specific root**.

Verification therefore resolves the issuer as: explicit operator override → `rootIssuer[R]` → the document's claim, and reports which path was used.
This also means an **old credential resolves against the clone that issued it**, never against a successor: if a clone is superseded, credentials it issued remain legitimate and verification must not silently re-point.
A document/chain disagreement is reported, never silently followed.

**This binds every surface, including the phones.**
A holder app that hands its resolver the document's `documentStore` leaves the attack open on the surface a border official actually holds: point it at *another authority's* real, factory-deployed clone and link 1 passes — that address genuinely is a clone — so the phone renders that authority's on-chain name, its claimed domain, and a green DNS badge.
Both apps read `rootIssuer[R]` first (`RoaxRpc.rootIssuer`, three-state: a real address / the zero address meaning "no record" / a failed read), follow it when it resolves, and render "The chain records a different issuing contract than this document names" when it disagrees with the document.
A **failed** `rootIssuer` read is not "no record": it reports `unavailable`, rather than becoming a licence to trust the document's claim unchecked.

**A definite link-1 failure fails the verdict.**
`isValid` is read from the resolved issuer, which falls back to the document's claim when the factory has no record of the root — so an attacker's own contract can answer it however it likes.
Reporting `notFactoryDeployed` beside `verdict: true` is worse than not checking: it is checked, failed, and passed anyway.
Only the **definite** negative fails; an unread provenance (no factory configured, or the read failed) is evidence of nothing and leaves the verdict alone, exactly as `couldNotCheck` is never treated as `notListed`.

## The states

Six states, and no two may be collapsed.

| State | Meaning | Register |
|---|---|---|
| `verified` | The records were fetched and the record exists. Carries the domain's own description. | positive |
| `notADogTagIssuer` | **Link 1 failed.** The contract did not come from the DogTag factory. | negative |
| `notListed` | Links 1 and 2 hold; the records were fetched and the address is absent. | negative |
| `couldNotCheck` | A real DNS attempt failed. **Evidence of nothing.** | neutral |
| `noDomainClaimed` | Link 1 holds; the issuer has claimed no domain. Normal on day one. | neutral |
| `unavailable` | A prerequisite could not be read at all (nothing configured, or the read failed). | neutral |

Plus a client-only `pending` while resolution is in flight.

**`notADogTagIssuer` must never be softened into `notListed`.**
It is a categorically stronger statement about a different thing, and it gets its own copy and its own mark.

**`couldNotCheck` must never be coloured as a failure.**
A resolver timeout says nothing whatsoever about the issuer.
Collapsing it into either neighbour is the fail-open bug this feature exists to remove — the same shape as `.unknown -> VALID` in the mobile importer.

The three "we do not know" states share a neutral treatment but have **different text**.
That is not the forbidden collapse: what is forbidden is letting `couldNotCheck` read as `verified` or as `notListed`, or letting a provenance failure read as a DNS one.

### The DoH classification rule

One rule, mirrored in `dogtag-dns-rs` (`classify_txt`), Swift (`IssuerBindingResolver.classifyDoh`) and Kotlin (`IssuerBindingResolver.classifyDoh`):

| DoH `Status` | Answers | State |
|---|---|---|
| `0` NOERROR | a TXT record at the name | `verified` |
| `0` NOERROR | no TXT (NODATA, or only CNAMEs) | `notListed` |
| `3` NXDOMAIN | — | `notListed` (a **definitive** absence) |
| `2` SERVFAIL, `5` REFUSED, anything else | — | `couldNotCheck` |
| absent / non-numeric | — | `couldNotCheck` |

An **absent** `Status` is not "no error": it means the body is not a DoH answer, and reading it as NOERROR would turn a wrong-endpoint misconfiguration into a confident "absent".

TXT RDATA is a sequence of character-strings, each at most 255 octets, which DoH renders as space-separated quoted chunks.
RFC 1035 says a consumer **concatenates** them: `"abc" "def"` is `abcdef`, not `abc def`.
Getting this wrong corrupts any description over 255 bytes.

## Where the DNS lookup happens

Browsers and mobile apps cannot do raw DNS, so this is a deliberate choice with a privacy cost either way.

**Web portals resolve server-side, in the stack's own API.**
A client-side DoH call would hand a public resolver the operator's IP plus which issuer they are inspecting.
The portal's backend already holds the whole credential, so resolving there adds no party to the trust set.
Endpoint is `DNS_DOH_ENDPOINT` (default Cloudflare).

**Mobile resolves DNS-over-HTTPS directly from the device, with no DogTag server in the loop.**
The alternative — routing through our own API — would tell *us* which credential the holder is viewing and when, which is strictly worse for a privacy-sensitive wallet than telling a public resolver a domain name.
It also matches the existing precedent: the phone already resolves DoH directly for the pre-disclosure groomer binding.

The cost, stated plainly: **Cloudflare learns the issuer's domain and the device's IP at the moment a credential is viewed.**
It does not learn the credential, the holder, or the dogTagId.

## Block anchoring, and the chain/DNS asymmetry

DNS records change and clones get superseded, so a verdict without a "when" is not auditable — and this is an audit product.

Every on-chain read in one verification is **pinned to a single block**, read once up front, and that block is reported.
`IssuerDomainRegistry.Binding` additionally stores `updatedAtBlock`, so "what domain did this clone claim at block N" is answerable rather than only "what does it claim now".

**The asymmetry that governs what a result may claim:**

- **Chain state is reproducible.** Anyone with an archive node can re-run every read pinned to the reported block and get the same answer. Verified empirically against the ROAX node (Geth v1.15.10): `eth_call name()` at head−5000 returns full historical state, and `eth_getStorageAt` at head−100000 answers without a missing-trie-node error. The anchor is not decorative.
- **DNS has no history.** There is no way to ask what a zone published at block N. A TXT record is only ever observable *now*.

So the DNS half is an **observation that can never be recomputed**, and it is labelled as such (`dnsObservation: "live" | "stored"`, `dnsHistorical: false`) alongside — never inside — the block anchor.
A stored observation must never be presented as live, and a live one must never be presented as proving the past.

`dnsObservation` is **derived from the observation's own `checkedAt`**, never stamped on.
The resolver replays a cached answer for up to `CacheTtl::answer_max` (15 min) keeping its ORIGINAL timestamp — deliberately, so a surface can say how old the answer really is — so a hardcoded `"live"` printed "DNS checked just now" over a quarter-hour-old observation.
The freshness window is **60 seconds**, the same threshold the TS renderer and both phones use, so the four legs agree on when "just now" stops being true.

## Who may write a binding

`DogTagIssuer` clones have **no owner**: they are `Initializable` only, and all write authority is `IssuerRegistry.isWhitelistedFor(recordType, msg.sender)`.
`IssuerDomainRegistry` therefore authorizes three tiers:

1. **`WHITELIST_ADMIN`** on the live `IssuerRegistry` — the protocol/KYC operator. Writes the initial value, because the admin onboarding flow already collects and DNS-checks `(domain, documentStore)`.
2. **The spawning business.** `createIssuer` salts a clone with `keccak256(recordType, business)` and never stores `business`, but `factory.predictIssuer(clone.recordType(), msg.sender) == clone` **proves** the claim. A one-way check (verify, not enumerate), which is all an authorization needs. This is the self-service tier.
3. **An appointed `domainAdmin[clone]`** — set once by `WHITELIST_ADMIN`, self-updating thereafter.

Tier 3 exists because tier 2 is only as strong as provisioning discipline.
`resolve_business` in the admin API **defaults `business` to the operator's own signer** when the caller omits it, and `scripts/demo-provision-government.sh` does exactly that.
For any clone spawned that way, tier 2 authorizes the **operator**, not the organisation, so the "two independent keys" property weakens to "protocol admin plus DNS".
Already-salted clones cannot be re-salted, so `domainAdmin` is how an existing issuer gets genuine self-service.
**New clones should be created with an explicit, organisation-controlled `business` address.**

### Why a new contract rather than a field on the clone

Both obvious alternatives are far more expensive than they look:

- **A field on `DogTagIssuer`** needs a new implementation, but `DogTagIssuerFactory.implementation` is `immutable` — so it needs a **new factory**. `VerificationRegistryConsent.rootIndex` is *also* `immutable` and resolves the issuing clone for every proof via `rootIndex.rootIssuer(R)`. A new factory leaves the live verification registry reading the old index, and every root issued by a new clone resolves to `address(0)`: owner-hidden consent verification breaks for every credential issued after the swap. That is a protocol-wide v2 plus a timelocked `ProtocolRegistry` republish, not a one-field change.
- **A mapping on `IssuerRegistry`** needs that registry redeployed (it is a plain, non-upgradeable AccessControl contract), and every existing clone pins `registry` at `initialize` with **no setter** — the clones would keep gating writes on the old registry while the domain lived in the new one. Split-brain governance.

`IssuerDomainRegistry` redeploys **nothing**. It is purely additive, like `ProtocolRegistry`.

It is deliberately **not** added to `ProtocolRegistry.ContractSet`: that struct is fixed, so a field there is itself a ProtocolRegistry redeploy plus a 2-day timelocked republish.
Apps discover it via `ISSUER_DOMAIN_REGISTRY_ADDR`, at the same trust level as `ISSUER_REGISTRY_ADDR` and the RPC URL.
It can join at the next ContractSet rotation.

## The separate DID assertion

DNS binding and the DID assertion catch **different** forgeries, and both are required.

`data.issuer` is root-covered and holds the true identity as a `did:web:` value.
Asserting `issuer.domain` against that DID's host catches a **domain** relabel and fails the verdict outright.

But `did:web:` carries a **domain, nothing else**.
Relabelling *only* `issuer.name` passes integrity, passes the DID assertion, **and** passes the DNS binding — the genuine domain really does publish the record.
Without a second source that attack renders a fabricated authority beside a green check, which is worse than showing nothing.

The second source is `DogTagIssuer.name()`, written by the factory's `onlyOwner` `createIssuer` at KYC time.
**That is the only issuer name a surface may present.**
The document's `issuer.name` is shown only to state a disagreement.
Name comparison normalises whitespace and case, because a padding difference is not evidence of anything and flagging it would train operators to ignore the flag.

`notAssertable` (the document carries no root-covered DID) neither passes nor fails: it is reported as un-asserted, never as verified.
A skipped pillar contributing a pass is exactly how an unverified claim reaches a user looking verified.

## Display rules

The badge is **one compact line beside the issuer**, not a panel.

State the **observation**, never a verdict:

- `verified` — a small green check plus "This address is listed in `<domain>`'s DNS records", **plus the description the domain published**.
- `notListed` — the symmetric red line: same size, same placement, same register, "This address is **not** listed in `<domain>`'s DNS records".
- `notADogTagIssuer` — red, its own mark, and it says nothing about DNS: "This contract was not deployed by the DogTag factory".
- `couldNotCheck` — neutral: "We could not reach DNS to check this domain".
- `noDomainClaimed` — neutral and unremarkable: "This issuer has published no domain on-chain".
- a document/chain issuer disagreement — red, and still an observation: "The chain records a different issuing contract than this document names". It says what the chain records; it passes no judgement on the credential.

**Never** write "VERIFICATION FAILED", "INVALID", "UNTRUSTED", "WARNING", or anything that reads as a judgement on the credential or the organisation.
A missing DNS record means the domain owner has not published the binding.
It does **not** mean the credential is bad — validity is separately and genuinely proven on-chain — and conflating the two would tell someone their perfectly valid credential failed, which is worse than showing nothing.

Absence of a record is a **normal** state.
Most issuers will not have one on day one, and it must look unremarkable.
An issuer shown *without* a badge must not read as verified, which is why `noDomainClaimed` gets a quiet line rather than silence.

A surface that renders before resolution completes shows `pending` and lets it resolve.
It must not pre-fill a guess: flashing "no domain claimed" and then flipping would show a state that was never observed.

## No synthesized results, anywhere

Every state a user sees is the outcome of a real resolution, or of a real resolution that really failed.
There is no stub, fixture, or demo shortcut in any shipped path.
Test doubles exist only in tests, behind an explicit transport seam.

Caching does not violate this: a cached entry is a real prior observation, and it keeps its **original** `checkedAt`, so a surface shows when DNS was really consulted rather than implying it just looked.
TTLs are asymmetric — a real answer is reused for up to 15 minutes (or the zone's own shorter TTL, floored at 60s), while a failure is reused for 30 seconds so a blip clears on its own.

The admin onboarding legitimacy gate follows the same rule.
It is **advisory**: it never blocks whitelisting, because an organisation is routinely KYC-approved days before its DNS team publishes anything, and a hard block drives operators toward a bypass.
What makes that safe is the trace — a non-verified observation requires the admin's explicit `proceedWithoutDns`, and both the observation and the fact that they proceeded are persisted (`dnsStateAtApproval`, `dnsProceededUnverified`), so an override is never indistinguishable from a clean pass.
`dnsState` / `dnsCheckedAt` are the mutable latest observation, so a future daily re-check job can flip a binding to verified with no admin redoing anything.

## Publishing a record — for issuers

1. Find your issuer contract address (the clone). The admin console shows it; it is also `issuer.documentStore` in any credential you issued.
2. Have your domain's DNS administrator add:

   ```
   Name:  <your-clone-address-in-lowercase>._dogtag
   Type:  TXT
   Value: any description you like, e.g. "Travel clearance issuance, IT dept"
   ```

   (Most DNS UIs take the name relative to your zone, so enter just the `<address>._dogtag` part.)
3. Confirm it resolves:

   ```
   dig +short TXT <address>._dogtag.<your-domain>
   ```
4. The badge picks it up within the cache TTL. The value is yours — verifiers display it verbatim and never parse it.

## Implementations

| Layer | Location |
|---|---|
| Contract | `contracts/src/IssuerDomainRegistry.sol` |
| Rust resolver (shared) | `crates/dogtag-dns-rs` |
| DID assertion (shared) | `crates/dogtag-standard-rs/src/issuer_identity.rs` |
| Server-side verification | `stacks/government/api/src/routes.rs` (`/v1/verify`) |
| Advisory onboarding gate | `stacks/admin/api/src/dns.rs`, `routes.rs` |
| Web badge + copy | `packages/ui/src/components/DomainBindingBadge.tsx`, `src/domain/issuerDomainBinding.ts` |
| iOS | `apps/ios/DogTag/IssuerDomainBinding.swift` |
| Android | `apps/android/.../net/IssuerDomainBinding.kt` |

The DoH classification rule is duplicated across Rust, Swift and Kotlin because each runs in a different process with no shared runtime.
All three carry the same unit tests over the same DoH bodies; **if you change one, change all three.**
