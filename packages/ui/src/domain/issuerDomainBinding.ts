/**
 * The issuer↔domain DNS TXT binding, as the portals consume it.
 *
 * Mirrors the Rust `dogtag-dns-rs` wire shape. See that crate for the normative record convention:
 *
 *   `<clone-address-lowercase>._dogtag.<domain>  IN TXT  "<free-form description>"`
 *
 * The address is in the NAME so the VALUE can stay free-form — the domain owner writes whatever
 * description they like, and presence of the record at that name IS the assertion.
 *
 * # What the badge may and may not claim
 *
 * A `verified` binding proves ONE thing: the owner of that DNS zone published a record naming this
 * contract address. It does NOT establish that the named organisation is who it says it is — that comes
 * from the separate KYC/whitelist root of trust. Copy must state the observation, never a verdict about
 * the organisation or the credential. Credential validity is proven on-chain and is independent of this.
 *
 * # The binding is a THREE-LINK CHAIN
 *
 * All three links are required, and each is re-checked at verification time rather than inherited from
 * a stored claim:
 *
 *  1. **Factory provenance** — the contract was deployed by the DogTag factory, so it passed through
 *     KYC-gated `createIssuer`. Without this the rest is worthless: anyone can deploy their own
 *     contract, claim a domain for it and publish a matching TXT record. DNS would agree, the registry
 *     would agree, and none of it would mean anything.
 *  2. **The on-chain domain claim** — read from the domain registry, never from the document.
 *  3. **The DNS half** — that domain's zone lists this contract address.
 *
 * # Why six states and not three
 *
 * The three DNS outcomes (`verified` / `notListed` / `couldNotCheck`) must never collapse into each
 * other — conflating a resolver failure with an absence is the fail-open bug this feature exists to
 * remove. Three more exist because they answer genuinely different questions:
 *
 *  - `notADogTagIssuer` — link 1 FAILED. A categorically stronger statement than a missing DNS record,
 *    and it must never be softened into "not listed".
 *  - `noDomainClaimed` — link 1 holds and this issuer has claimed no domain. Nothing to check. The
 *    NORMAL state on day one, and it must look unremarkable.
 *  - `unavailable` — a prerequisite could not be read at all (nothing configured, or the read failed).
 *
 * Visually the three "we do not know" states share a neutral treatment with DIFFERENT text. That is not
 * the forbidden collapse: what is forbidden is letting `couldNotCheck` read as `verified` or as
 * `notListed`, or letting a provenance failure read as a DNS one.
 */

/** The state machine, exactly as the API emits it. `pending` is client-only (see below). */
export type IssuerDomainBindingState =
  | "verified"
  /** Link 1 failed: the contract was not deployed by the DogTag factory. */
  | "notADogTagIssuer"
  | "notListed"
  | "couldNotCheck"
  | "noDomainClaimed"
  | "unavailable"
  /**
   * Client-only: the lookup is in flight. Never sent by the API.
   *
   * A surface that renders before the chain read and DNS resolution complete MUST show this rather than
   * pre-filling a guess — showing `noDomainClaimed` first and then flipping would be synthesizing a
   * state that was never observed.
   */
  | "pending";

export interface IssuerDomainBinding {
  state: IssuerDomainBindingState;
  /** The ON-CHAIN claimed domain that was queried. Never the document's `issuer.domain`. */
  domain?: string;
  /** The clone address whose binding was sought (lowercase). */
  cloneAddress?: string;
  /** The fully-qualified TXT name queried, so an operator can reproduce the lookup by hand. */
  txtName?: string;
  /** Free-form text the DOMAIN OWNER published. Present only when `state === "verified"`. */
  description?: string;
  /** Unix seconds of the real observation. A cached result keeps its original timestamp. */
  checkedAt?: number;
  /** Why the check could not run (`unavailable`), for an operator-facing tooltip. */
  detail?: string;
}

/** True only for a `verified` binding. Use this rather than hand-rolling a truthiness check. */
export function isDomainVerified(b: IssuerDomainBinding | null | undefined): boolean {
  return b?.state === "verified";
}

/** The visual register a state gets. Only `verified` is positive; only `notListed` is negative. */
export type BindingTone = "positive" | "negative" | "neutral" | "pending";

export function bindingTone(state: IssuerDomainBindingState): BindingTone {
  switch (state) {
    case "verified":
      return "positive";
    case "notListed":
      // Red plus a factual sentence, never an alarm. A missing record means the domain owner has not
      // published the binding; it says nothing about the credential, whose validity is separately
      // proven on chain.
      return "negative";
    case "notADogTagIssuer":
      // Also red, but its COPY is categorically different — see `bindingLine`. This is a definitive
      // observation about the contract's provenance, not about a DNS zone.
      return "negative";
    case "pending":
      return "pending";
    default:
      // `couldNotCheck`, `noDomainClaimed` and `unavailable` are all "we do not know". Deliberately
      // NEITHER green nor red: a resolver timeout is not evidence either way.
      return "neutral";
  }
}

/**
 * The one-line copy for a state. Every string states what was LOOKED AT and what was FOUND, and stops
 * there — no "VERIFICATION FAILED", no "INVALID", no "UNTRUSTED", nothing that reads as a judgement on
 * the credential or the organisation.
 */
export function bindingLine(b: IssuerDomainBinding): string {
  const domain = b.domain?.trim() || "this domain";
  switch (b.state) {
    case "verified":
      return `This address is listed in ${domain}'s DNS records`;
    case "notListed":
      return `This address is not listed in ${domain}'s DNS records`;
    case "notADogTagIssuer":
      // Deliberately says nothing about DNS: DNS was never the question here.
      return "This contract was not deployed by the DogTag factory";
    case "couldNotCheck":
      return "We could not reach DNS to check this domain";
    case "noDomainClaimed":
      return "This issuer has published no domain on-chain";
    case "unavailable":
      return "The on-chain domain claim could not be read";
    case "pending":
      return "Checking this domain's DNS records…";
  }
}

/**
 * The longer explanation, for a tooltip or a detail row. Says exactly what the check establishes and
 * what it deliberately does not.
 */
export function bindingExplanation(b: IssuerDomainBinding): string {
  switch (b.state) {
    case "verified":
      return (
        `The owner of ${b.domain} published a DNS TXT record naming this contract address. ` +
        "That shows the domain owner vouches for this address — it is not a check of the " +
        "organisation's identity, which comes from KYC at onboarding."
      );
    case "notListed":
      return (
        `${b.domain} publishes no DNS record for this contract address. Most issuers have not ` +
        "published one. This says nothing about the credential, whose validity is proven on-chain."
      );
    case "notADogTagIssuer":
      return (
        "The contract that issued this credential does not come from the DogTag factory, so it never " +
        "went through onboarding. A domain binding on such a contract would show only that whoever " +
        "deployed it also controls a domain."
      );
    case "couldNotCheck":
      return (
        "The DNS lookup did not complete, so we do not know whether the record exists. " +
        "This is evidence of nothing either way."
      );
    case "noDomainClaimed":
      return (
        "This issuer's contract has not claimed a domain on-chain, so there is nothing to check " +
        "against DNS. This is normal."
      );
    case "unavailable":
      return b.detail?.trim()
        ? `The on-chain domain claim could not be read: ${b.detail}`
        : "The on-chain domain claim could not be read.";
    case "pending":
      return "Reading the on-chain domain claim and resolving its DNS records.";
  }
}

/** The exact TXT record an issuer must publish, for operator-facing "how do I fix this?" copy. */
export function expectedTxtName(cloneAddress: string, domain: string): string {
  return `${cloneAddress.trim().toLowerCase()}._dogtag.${domain.trim().replace(/\.$/, "").toLowerCase()}`;
}

// -------------------------------------------------------------------------------------------------
// The issuer identity that accompanies the badge
// -------------------------------------------------------------------------------------------------

/**
 * The issuer identity as `/v1/verify` reports it.
 *
 * `onchainName` is AUTHORITATIVE: it is `DogTagIssuer.name()`, written by the factory's `onlyOwner`
 * `createIssuer` at KYC time. `documentName`/`documentDomain` come from the credential's `issuer` block,
 * which is OUTSIDE the Merkle root — a genuine credential can be re-badged as any authority and still
 * pass integrity. Never render the document values as the issuer; show them only to state a
 * disagreement.
 */
export interface IssuerIdentity {
  onchainName?: string | null;
  onchainNameAvailable?: boolean;
  documentName?: string;
  documentDomain?: string;
  rootCoveredDomain?: string | null;
  /** "match" | "mismatch" | "notAssertable" — never a boolean. */
  assertion?: string;
  documentNameDiffers?: boolean;
  documentDomainDiffers?: boolean;
  relabelled?: boolean;
}

/**
 * The issuer name a surface should display, and whether it is the authoritative one.
 *
 * Falls back to the document's name ONLY when the chain could not be read, and says so — so a fallback
 * is never presented as authoritative.
 */
export function displayIssuerName(id: IssuerIdentity | null | undefined): {
  name: string;
  authoritative: boolean;
} {
  const onchain = id?.onchainName?.trim();
  if (id?.onchainNameAvailable && onchain) return { name: onchain, authoritative: true };
  return { name: id?.documentName?.trim() || "Unknown issuer", authoritative: false };
}
