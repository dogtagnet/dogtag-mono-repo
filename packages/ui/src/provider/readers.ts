/**
 * The chain seam the provider self-service engine reads through (registry-plan S-15).
 *
 * ONE rule governs the whole of this file, and it is the rule that fixed the issuer-whitelist
 * pillar:
 *
 *   > An address may be an ARGUMENT TO A WRITE, checked against the registry's own factory
 *   > reference. An address may never be an ARGUMENT TO A READ THAT DECIDES TRUST - there it must
 *   > be resolved from configuration or from the chain.
 *
 * So a reader is CONSTRUCTED with the core, factory, resolver and directory it speaks for, and no
 * method takes a factory, core or resolver address as a parameter. A candidate clone is a
 * parameter, because the whole point of the guard is to judge one - but the contract that judges it
 * is never the caller's to nominate. Handing the reader a factory address per call is precisely how
 * a forger would supply both sides of the question.
 *
 * Reads THROW on failure and never return a sentinel. The engine catches and reports
 * `could-not-run`; a reader that swallowed a fault and answered `false` would turn "we could not
 * ask" into "the chain refused you", which is the defect this surface exists not to have.
 */

import type { IssuerCreationLog } from "./deploymentHistory";

/** A 0x-prefixed hex address. */
export type Address = `0x${string}`;
/** A 0x-prefixed hex word (bytes32 or, for a provider id, bytes20). */
export type HexWord = `0x${string}`;

/** `ProviderRegistry.Standing`, in declaration order. Read a number, never a name, off the chain. */
export enum Standing {
  NONE = 0,
  PENDING = 1,
  ACTIVE = 2,
  SUSPENDED = 3,
  RETIRED = 4,
}

export const STANDING_LABEL: Readonly<Record<Standing, string>> = {
  [Standing.NONE]: "not registered",
  [Standing.PENDING]: "pending review",
  [Standing.ACTIVE]: "active",
  [Standing.SUSPENDED]: "suspended",
  [Standing.RETIRED]: "retired",
};

/** `ProviderRegistry.Service`, as read back. */
export interface ServiceRecord {
  /** bytes20. All-zero means this address is attached to no provider at all. */
  providerId: HexWord;
  factoryGeneration: HexWord;
  recordType: HexWord;
  confirmedOwner: Address;
  domainResolver: Address;
  ownerEpoch: bigint;
  standing: Standing;
}

/** `ProviderRegistry.Provider`, trimmed to what this surface reads. */
export interface ProviderRecord {
  controller: Address;
  directoryResolver: Address;
  standing: Standing;
}

/** `ProviderRegistry.effectiveService`'s five separately-reported terms. */
export interface EffectiveService {
  providerStanding: Standing;
  serviceStanding: Standing;
  factoryActive: boolean;
  ownerConfirmed: boolean;
  hasActiveIssuer: boolean;
}

/** `ServiceDomainResolver.Disposition`, in declaration order. */
export enum DomainDisposition {
  UNSET = 0,
  NO_DOMAIN = 1,
  CLAIMED = 2,
  CLEARED = 3,
}

/** `ServiceDomainResolver.claimStanding`'s six separately-reported terms. */
export interface DomainClaimStanding {
  disposition: DomainDisposition;
  domain: string;
  lineageRecognizesService: boolean;
  registryApprovesThisResolver: boolean;
  coreSelectsThisResolver: boolean;
  /**
   * NOT a would-this-write-succeed answer. `true` means only "not frozen" - it discards the
   * confirmed-owner term, so a service quarantined by an unconfirmed clone-owner handover reads
   * `true` here while every write is refused. Read it alongside `canWriteDomain`, never instead.
   */
  serviceStandingEffective: boolean;
}

/**
 * `ProviderRegistry.ResolverKind`, in declaration order.
 *
 * The two kinds are keyed differently and that difference is the whole reason there are two
 * selections rather than one: a DIRECTORY resolver is chosen per PROVIDER (one listing, however many
 * contracts the provider holds), a DOMAIN resolver is chosen per SERVICE (a domain belongs to one
 * contract). Read a number off the chain, never a name.
 */
export enum ResolverKind {
  DIRECTORY = 0,
  DOMAIN = 1,
}

/**
 * One entry of a kind's resolver list, with the authoritative approval bit beside it.
 *
 * **THE LIST IS APPEND-ONLY AND KEEPS DEAPPROVED ENTRIES, WHICH IS WHY THIS IS A PAIR AND NOT AN
 * ADDRESS.** `ProviderRegistry.setResolverApproved` pushes onto `_resolverAddresses[kind]` the first
 * time it sees an address and NEVER removes it - pulling a resolver only flips
 * `_approvedResolvers[kind][resolver]` to false. So `resolverPage` answers "every resolver this
 * registry has ever named for this kind", which is not the same question as "what may I choose".
 *
 * Offering a deapproved entry as a choice is a button whose write reverts `ResolverNotApproved`, and
 * the mistake is invisible on today's chain: each kind has exactly one entry and both are approved,
 * so a list-only implementation and a filtered one agree on every value the deployment can produce.
 * The admin console's `GET /v1/admin/resolvers` already returns `{resolver, approved}` pairs for the
 * same reason; this mirrors that shape rather than inventing a second one.
 */
export interface ResolverListing {
  resolver: Address;
  approved: boolean;
}

export const ZERO_PROVIDER_ID: HexWord = `0x${"0".repeat(40)}`;
export const ZERO_ADDR: Address = `0x${"0".repeat(40)}`;
/** A `bytes32` that has never been written. `ProfileAnchor.digest` at revision 0. */
export const ZERO_WORD: HexWord = `0x${"0".repeat(64)}`;

/** One published location, as `ProviderDirectory.pin` returns it. Coordinates are already scaled. */
export interface DirectoryPin {
  locationNo: number;
  /** Scaled by `COORDINATE_SCALE`, exactly as the contract stores it. */
  lat: number;
  lng: number;
  kind: number;
  active: boolean;
}

/** `ProviderDirectory.ProfileAnchor`, trimmed to what deciding a re-anchor needs. */
export interface ProfileAnchorRecord {
  /** All-zero means no anchor is currently published - which, at revision 0, means never published. */
  digest: HexWord;
  schema: number;
  codec: number;
  hashAlgorithm: number;
  revision: bigint;
}

/**
 * Every chain read the four S-15 flows need.
 *
 * Injected so the engine is testable without a node, and so a live implementation can be swapped
 * for a block-pinned one without the decision logic noticing.
 */
export interface ProviderChainReader {
  // -- provenance and control ------------------------------------------------------------------

  /**
   * `factory.isClone(candidate)` against THIS READER'S configured factory.
   *
   * The single forgery predicate. That mapping is written in exactly one place - `createIssuer` -
   * so an address is in it if and only if this factory deployed it.
   */
  isFactoryClone(candidate: Address): Promise<boolean>;

  /**
   * `IOwnable(candidate).owner()`.
   *
   * Expected to THROW for an address with no code. An EOA staticcall SUCCEEDS with empty
   * returndata rather than reverting, which is the silent shape a probe must reject - a client
   * that decodes empty returndata as the zero address would report an EOA as "owned by nobody"
   * instead of "not answerable".
   */
  cloneOwner(candidate: Address): Promise<Address>;

  // -- the core --------------------------------------------------------------------------------

  service(candidate: Address): Promise<ServiceRecord>;
  effectiveService(candidate: Address): Promise<EffectiveService>;
  provider(providerId: HexWord): Promise<ProviderRecord>;
  currentService(providerId: HexWord, recordType: HexWord): Promise<Address>;

  /**
   * `ProviderRegistry.canWriteService(service, caller, SERVICE_PERMISSION_REPOINT)` - the EXACT
   * predicate `repointService` is gated by (`ProviderRegistry.sol:653-656`).
   *
   * COMPOSED, never re-derived, and that is the point of this method existing. The chain admits the
   * confirmed live owner OR an owner-epoch-scoped delegate holding that permission bit, so a portal
   * that asked only `owner() == caller` would be STRICTER than the contract and would tell a
   * legitimate delegate its key may not select a contract the chain would happily let it select.
   * That is the same shape as `canCreateService`'s `msg.sender` trap above, pointed the other way,
   * and this repo already records what two parallel implementations of one authorization rule cost.
   *
   * The permission bit is bound by the implementation rather than passed by the caller: a caller
   * free to name the bit is a caller free to name the wrong one, and every bit here answers a
   * different question.
   */
  canWriteServiceRepoint(service: Address, caller: Address): Promise<boolean>;

  /**
   * `ProviderRegistry.canCreateService(providerId, recordType, caller)`.
   *
   * READ THIS ONE CAREFULLY. Its first term is `generationOfFactory[msg.sender]` - the CALLING
   * factory - because the core is designed to be asked this BY a factory during `createIssuer`. A
   * plain `eth_call` carries no `from`, so `msg.sender` is the zero address, no generation matches,
   * and the answer is `false` for every provider on earth. That is the same `msg.sender`-branch
   * trap `ProviderRegistry.isWhitelistedFor` carries, and it fails in the more dangerous direction
   * here: a preflight would tell a perfectly eligible provider it may not deploy.
   *
   * An implementation MUST therefore make this call with `from` set to the configured factory. The
   * live implementation does; `providerCanCreateServiceIsAskedAsTheFactory` is the pin.
   *
   * It cannot be decomposed instead: the service-creation approval it folds is stored in a PRIVATE
   * mapping with no getter, so this aggregate is the only way to observe that term at all.
   */
  canCreateService(providerId: HexWord, recordType: HexWord, caller: Address): Promise<boolean>;

  /** `factory.predictIssuer(recordType, business, cloneNonce)` - exact, before the transaction. */
  predictIssuer(recordType: HexWord, business: Address, cloneNonce: bigint): Promise<Address>;

  /**
   * Every `IssuerOwnerRegistered` this factory has emitted for `owner` - what that wallet deployed.
   *
   * Filtered by OWNER and never by provider id. The factory's salt is
   * `keccak256(abi.encode(recordType, msg.sender, cloneNonce))`, so the owner is what shares a
   * number's namespace and the provider id is not in it at all; see `deploymentHistory.ts`.
   *
   * The event carries no record type, which is why {@link cloneRecordType} exists rather than a
   * second, dependent log filter.
   */
  issuerCreations(owner: Address): Promise<readonly IssuerCreationLog[]>;

  /**
   * `DogTagIssuer.recordType()` on the clone itself.
   *
   * Deliberately the CLONE's own immutable value rather than `service(clone).recordType`: the core's
   * copy is written by `attachService` and is all-zero until the registrar acts, so reading it would
   * report no record type for exactly the freshly-deployed contract this is asked about.
   */
  cloneRecordType(clone: Address): Promise<HexWord>;

  // -- the typed resolvers ---------------------------------------------------------------------

  /** `ServiceDomainResolver.claimStanding(service)`. */
  domainClaimStanding(service: Address): Promise<DomainClaimStanding>;
  /** `ServiceDomainResolver.canWriteDomain(service, who)`. */
  canWriteDomain(service: Address, who: Address): Promise<boolean>;

  /** `ProviderDirectory.isLiveFor(providerId)` - approved fleet-wide AND selected by this provider. */
  directoryIsLiveFor(providerId: HexWord): Promise<boolean>;
  /** `ProviderRegistry.canWriteProvider(providerId, caller, PROVIDER_PERMISSION_RECORD)`. */
  canWriteProviderRecord(providerId: HexWord, caller: Address): Promise<boolean>;

  // -- resolver selection ----------------------------------------------------------------------

  /**
   * Every address this registry has ever named for `kind`, each with `isResolverApproved` beside it.
   *
   * PAGED TO COMPLETION, not first-page-only: `resolverCount` bounds the walk and a partial read is a
   * failure rather than a shorter list, because a resolver missing from a half-read list is
   * indistinguishable from one the registrar never approved - and the provider would be told their
   * only legitimate choice does not exist.
   *
   * The approval bit comes from {@link ResolverListing}, whose doc says why it cannot be dropped.
   */
  approvedResolvers(kind: ResolverKind): Promise<readonly ResolverListing[]>;

  /**
   * `ProviderRegistry.canWriteProvider(providerId, caller, PROVIDER_PERMISSION_DIRECTORY_RESOLVER)`
   * - the EXACT predicate `setDirectoryResolver` is gated by.
   *
   * A DIFFERENT method from {@link canWriteProviderRecord} rather than one method taking the bit,
   * for the reason the whole of this file gives: a caller free to name the permission is a caller
   * free to name the wrong one, and every bit answers a different question. Publishing content into
   * a listing and choosing which register holds that listing are separate grants, so a delegate may
   * hold either without the other.
   */
  canWriteProviderDirectoryResolver(providerId: HexWord, caller: Address): Promise<boolean>;

  /**
   * `ProviderRegistry.canWriteService(service, caller, SERVICE_PERMISSION_DOMAIN_RESOLVER)` - the
   * EXACT predicate `setDomainResolver` is gated by.
   *
   * Distinct from {@link canWriteServiceRepoint} for the same reason, and the numbers make the
   * hazard concrete: this bit is `1 << 1` and the repoint bit is `1 << 2`, so a copied constant is a
   * confident `false` about a permission nobody asked about.
   */
  canWriteServiceDomainResolver(service: Address, caller: Address): Promise<boolean>;

  /**
   * `ProviderDirectory.profileAnchor(providerId)`.
   *
   * Read so a publication that would change nothing can be left unsent. `setProfileAnchor` has NO
   * `NoChange` guard and the contract is frozen, so the portal is the only place a no-op re-anchor
   * can be avoided - and a needless revision bump silently invalidates
   * `coversCurrentAddressText` on any registrar address confirmation the provider holds.
   */
  providerProfileAnchor(providerId: HexWord): Promise<ProfileAnchorRecord>;

  /** `ProviderDirectory.pinCount(providerId)` - how many locations this provider has published. */
  providerPinCount(providerId: HexWord): Promise<number>;
  /** `ProviderDirectory.nextLocationNumber(providerId)` - the number the NEXT publish would take. */
  providerNextLocationNumber(providerId: HexWord): Promise<number>;
  /** `ProviderDirectory.hasPin(providerId, locationNo)`. */
  providerHasPin(providerId: HexWord, locationNo: number): Promise<boolean>;
  /** `ProviderDirectory.pin(providerId, locationNo)`. Throws `UnknownLocation` for an absent one. */
  providerPin(providerId: HexWord, locationNo: number): Promise<DirectoryPin>;
}
