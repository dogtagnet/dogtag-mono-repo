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

export const ZERO_PROVIDER_ID: HexWord = `0x${"0".repeat(40)}`;
export const ZERO_ADDR: Address = `0x${"0".repeat(40)}`;

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

  // -- the typed resolvers ---------------------------------------------------------------------

  /** `ServiceDomainResolver.claimStanding(service)`. */
  domainClaimStanding(service: Address): Promise<DomainClaimStanding>;
  /** `ServiceDomainResolver.canWriteDomain(service, who)`. */
  canWriteDomain(service: Address, who: Address): Promise<boolean>;

  /** `ProviderDirectory.isLiveFor(providerId)` - approved fleet-wide AND selected by this provider. */
  directoryIsLiveFor(providerId: HexWord): Promise<boolean>;
  /** `ProviderRegistry.canWriteProvider(providerId, caller, PROVIDER_PERMISSION_RECORD)`. */
  canWriteProviderRecord(providerId: HexWord, caller: Address): Promise<boolean>;
}
