import {
  BaseError,
  createPublicClient,
  encodeFunctionData,
  ExecutionRevertedError,
  keccak256,
  toBytes,
  type Abi,
  type Address,
  type PublicClient,
} from "viem";
import { guardedRoaxTransport } from "../chain/rpcEndpoint";
import { roax } from "./chain";

/**
 * The deployed contract set on ROAX (chainId 135). ONE set - there is no second generation to choose
 * between, and no runtime fork picks between an old and a new address.
 *
 * These are DEFAULTS. A `VITE_*` override that is set wins; one that is unset falls back here rather
 * than failing closed, so check which kind of variable you are editing before assuming an unset value
 * disables a read. `make check-cutover-consumers` is the gate over the whole tree.
 */
export const DEPLOYED_ADDRESSES = {
  /**
   * The provider authority. Holds provider and service records, the issuance capability the issuer's
   * `onlyIssuanceCapable` consults, and the orthogonal verifier capability. Deployed as
   * `ProviderRegistry`.
   */
  ProviderRegistry: "0xA4916d75722cf7d39a8E030cFbAee30a411aAEa9",
  DogTagSBT: "0xBEbc45A838643D27004827b797b30A464b2b02c0",
  /**
   * The owner-hidden verification registry. Its `Verified(uint256 indexed dogTagId, ...)` is the only
   * event that indexes a tag id, so it is what on-chain tag discovery must read. Its `rootIndex` is
   * the provenance router below, and its `issuerRegistry` is the provider authority above - both
   * immutable, so neither can be repointed without redeploying this contract.
   */
  VerificationRegistryConsent: "0xE49f30D6677f019f11298B3294377E6B817d43Da",
  /**
   * The ROOT INDEX: what `rootIssuer(R)` is asked of, and the address a READER resolving a credential
   * to its issuing clone must use. Not the factory - see `DogTagIssuerFactory` below.
   */
  CloneProvenanceRouter: "0xf374f4cA5ebBBAFf0dFcE48D8Cda2e47F9D5da01",
  /**
   * The clone factory: what a WRITER calls for `predictIssuer`/`createIssuer`. It deploys; it does
   * not resolve historical roots. Reading `rootIssuer` off it instead of off the router above
   * resolves only clones it deployed and answers the zero address for every other, which surfaces as
   * an indeterminate issuer pillar rather than an error.
   */
  DogTagIssuerFactory: "0x4CBfF4Cf47c313C9Df9689dd2A47eC71675233c6",
  /** The issuer implementation every clone delegates to. */
  DogTagIssuerImpl: "0x91E210AD9A5CCe4aF2C49221f0584E2ad6d13691",
  /** The discovery anchor. */
  ProtocolRegistry: "0xe98BFf66367F74F413414228adD91c16A24F7fdb",
  /** The typed DIRECTORY resolver, selected per provider through the authority. */
  ProviderDirectory: "0x25a318a0Bf83a7ea64fB0a7b1cDe8847722C7bC0",
  /** The typed DOMAIN resolver, selected per service through the authority. */
  ServiceDomainResolver: "0x4AB4a70CFa9CE9415B96dF543C218F90a2619c33",
  /** The frozen consent ceremony verifier. Unchanged - the same VK the bundled zkey proves against. */
  Groth16VerifierConsent: "0x1A9027986B859dc3879896B053deA78F636BE9b1",
  Poseidon6: "0x58091F2320c78ed6c6D1C02CB7E5c7578f1349db",
  admin: "0x8E27E117663bc6B65F82cC6E98412b4003e6F4A2",
} as const;

/** keccak256(recordType utf8 bytes) — matches the backend's record_type_key + IssuerRegistry. */
export function recordTypeKey(recordType: string): `0x${string}` {
  return keccak256(toBytes(recordType));
}

const ISSUER_REGISTRY_ABI = [
  {
    type: "function",
    name: "isWhitelistedFor",
    stateMutability: "view",
    inputs: [
      { name: "rt", type: "bytes32" },
      { name: "s", type: "address" },
    ],
    outputs: [{ name: "", type: "bool" }],
  },
] as const satisfies Abi;

/**
 * The generation-2 authority (`contracts/src/ProviderRegistry.sol`), declared for ONE purpose.
 *
 * `isRecognizedIssuer` is the one selector a generation-1 `IssuerRegistry` provably does NOT
 * implement — that contract's entire external surface is `whitelistFor` / `delistFor` /
 * `isWhitelistedFor`, and it has no fallback, so the call reverts there rather than answering. That
 * makes it usable as a GENERATION DISCRIMINATOR, which is the only thing this module uses it for.
 *
 * Its BOOLEAN is never consumed. `isRecognizedIssuer` reads current storage
 * (`_issuanceCapabilities[service][signer]`) with no block and no root, so it cannot answer "was this
 * in force when that root was anchored" — using its value would revert the pillar to a current-state
 * getter under a new name, which is the exact regression the forward-only rule removed.
 */
const PROVIDER_AUTHORITY_ABI = [
  {
    type: "function",
    name: "isRecognizedIssuer",
    stateMutability: "view",
    inputs: [
      { name: "service", type: "address" },
      { name: "signer", type: "address" },
    ],
    outputs: [{ name: "", type: "bool" }],
  },
] as const satisfies Abi;

const DOGTAG_ISSUER_ABI = [
  {
    type: "function",
    name: "isValid",
    stateMutability: "view",
    inputs: [{ name: "r", type: "bytes32" }],
    outputs: [{ name: "", type: "bool" }],
  },
  {
    type: "function",
    name: "isRevoked",
    stateMutability: "view",
    inputs: [{ name: "r", type: "bytes32" }],
    outputs: [{ name: "", type: "bool" }],
  },
  {
    // public `mapping(bytes32 => uint256) issuedAt` getter - 0 = not issued (DogTagIssuer.sol).
    type: "function",
    name: "issuedAt",
    stateMutability: "view",
    inputs: [{ name: "r", type: "bytes32" }],
    outputs: [{ name: "", type: "uint256" }],
  },
  {
    // public `mapping(bytes32 => address) issuedBy` getter - the H-1 originator (DogTagIssuer.sol).
    type: "function",
    name: "issuedBy",
    stateMutability: "view",
    inputs: [{ name: "r", type: "bytes32" }],
    outputs: [{ name: "", type: "address" }],
  },
  {
    // public `bytes32 recordType` getter - the clone's own immutable record type, fixed by the
    // factory at `createIssuer`. The authoritative answer to "what kind of credential is this?",
    // as opposed to the document's own claim.
    type: "function",
    name: "recordType",
    stateMutability: "view",
    inputs: [],
    outputs: [{ name: "", type: "bytes32" }],
  },
  {
    // public `IssuerRegistry registry` getter - the registry whose `isWhitelistedFor` this clone's
    // own `onlyWhitelisted` modifier consults (`DogTagIssuer.sol:40`). Written once at `initialize`
    // from the factory's immutable `registry`, and there is no setter: it is THE authority that
    // gates writes to this contract, and the only registry whose answer about its signers means
    // anything.
    type: "function",
    name: "registry",
    stateMutability: "view",
    inputs: [],
    outputs: [{ name: "", type: "address" }],
  },
] as const satisfies Abi;

/**
 * The authority's issuance-grant event, indexed on exactly the pair the question is asked about - so
 * the full history for one `(service, signer)` is ONE filtered `eth_getLogs`, with no scan and no
 * per-log decode of irrelevant entries.
 *
 * Keyed on the SERVICE ADDRESS, not a record-type key. A clone carries exactly one record type, so
 * filtering by service inherently scopes the history to it; the separate check that the DOCUMENT's
 * claimed record type matches `recordType()` stays at the caller, where a relabelled credential is
 * refused. `allowed` is the one NON-indexed argument, so grant and withdrawal arrive on one topic.
 */
const ISSUANCE_CAPABILITY_EVENT_ABI = [
  {
    type: "event",
    name: "IssuanceCapabilitySet",
    inputs: [
      { name: "service", type: "address", indexed: true },
      { name: "signer", type: "address", indexed: true },
      { name: "allowed", type: "bool", indexed: false },
    ],
  },
] as const satisfies Abi;

/** `DogTagIssuer.RootIssued`, indexed on the root - the anchoring event, and so the issuance BLOCK. */
const ROOT_ISSUED_EVENT_ABI = [
  {
    type: "event",
    name: "RootIssued",
    inputs: [
      { name: "root", type: "bytes32", indexed: true },
      { name: "by", type: "address", indexed: true },
      { name: "ts", type: "uint256", indexed: false },
    ],
  },
] as const satisfies Abi;

const DOGTAG_ISSUER_FACTORY_ABI = [
  {
    // public `mapping(bytes32 => address) rootIssuer` getter - the protocol-global, write-once
    // root -> issuing clone index (DogTagIssuerFactory.sol).
    type: "function",
    name: "rootIssuer",
    stateMutability: "view",
    inputs: [{ name: "root", type: "bytes32" }],
    outputs: [{ name: "", type: "address" }],
  },
] as const satisfies Abi;

/** The all-zero address `issuedBy` returns for a root this clone never issued. */
export const ZERO_ADDRESS = "0x0000000000000000000000000000000000000000";

/** The all-zero word a `bytes32` getter returns for an unset slot (e.g. an uninitialized clone). */
export const ZERO_BYTES32 = `0x${"0".repeat(64)}`;

/**
 * ## Why every reader below takes an optional `blockNumber`
 *
 * Against a MUTABLE world a verdict without a block anchor is not auditable: clones get superseded and
 * domain claims get rewritten, so "this root resolved to clone X" is only reproducible if it also says
 * WHEN. Pinning a batch of reads to one height additionally makes them a consistent SNAPSHOT rather than
 * several answers smeared across blocks, which is what lets two reads in the same verdict be compared at
 * all. This mirrors the government verify route, which reads the head once into `at_block` and pins the
 * whole verification to it (`stacks/government/api/src/routes.rs`).
 *
 * Omitting it reads `latest`, which is what every pre-existing caller does and continues to do - the
 * parameter is purely additive. A caller that could not read the head MUST omit it and report the answer
 * as unanchored; stamping an unpinned read with a separately-read head number would be claiming a
 * snapshot that was never taken.
 */
const clientCache = new Map<string, PublicClient>();

/**
 * A cached viem public client for ROAX.
 *
 * Its transport checks `eth_chainId` immediately before every read. A bad preferred endpoint gets no
 * address-bound request; the independently guarded bundled default is used instead.
 */
export function roaxPublicClient(
  rpcUrl?: string,
  defaultRpcUrl: string = roax.rpcUrls.default.http[0],
): PublicClient {
  const preferred = rpcUrl ?? defaultRpcUrl;
  const key = `${preferred}\n${defaultRpcUrl}`;
  let c = clientCache.get(key);
  if (!c) {
    c = createPublicClient({
      chain: roax,
      transport: guardedRoaxTransport(preferred, defaultRpcUrl),
    });
    clientCache.set(key, c);
  }
  return c;
}

/**
 * Arguments for {@link isWhitelistedFor}. Exactly ONE of `recordTypeKey` or `recordType` must be
 * supplied; the union makes neither (and both) a compile error. The registry answers a definite
 * `false` for any key no clone holds, so a caller that names no record type would get a confident
 * verdict on a question it never asked - the same "definite false for the wrong reason" the
 * zero-address `issuedBy` case is deliberately routed to indeterminate instead.
 */
export type IsWhitelistedForArgs = {
  registryAddr: string;
  address: string;
  rpcUrl?: string;
  defaultRpcUrl?: string;
  /** Pin this `eth_call` to a block height; omitted reads `latest`. See {@link roaxPublicClient}. */
  blockNumber?: bigint;
} & (
  | { recordTypeKey: string; recordType?: never }
  | { recordType: string; recordTypeKey?: never }
);

/**
 * Reads IssuerRegistry.isWhitelistedFor(key, address).
 *
 * Pass `recordTypeKey` when the key came from the chain (a clone's own `recordType()`); pass
 * `recordType` to hash a label locally. The key form is what verification uses - the record type it
 * asks about must be the one the chain says the root has, not one read off the document. Supplying
 * neither throws rather than inventing an empty key: refusing to ask is honest.
 */
export async function isWhitelistedFor(args: IsWhitelistedForArgs): Promise<boolean> {
  const key =
    args.recordTypeKey ??
    (args.recordType === undefined ? undefined : recordTypeKey(args.recordType));
  if (key === undefined) {
    throw new Error(
      "isWhitelistedFor: exactly one of recordTypeKey or recordType is required - refusing to ask the registry about keccak256(\"\")",
    );
  }
  return roaxPublicClient(args.rpcUrl, args.defaultRpcUrl).readContract({
    address: args.registryAddr as Address,
    abi: ISSUER_REGISTRY_ABI,
    functionName: "isWhitelistedFor",
    args: [key as `0x${string}`, args.address as Address],
    blockNumber: args.blockNumber,
  }) as Promise<boolean>;
}

/**
 * Reads DogTagIssuer.isValid(merkleRoot) — true once the Merkle root has been anchored on-chain
 * (issuedAt[root] != 0). The portal issue-status poller uses this to transition Anchoring →
 * Verified on-chain. `issuerAddr` is the per-recordType issuer contract (the prepare response's
 * unsignedTx.to in wallet mode).
 */
export async function isRootValid(args: {
  issuerAddr: string;
  root: string;
  rpcUrl?: string;
  defaultRpcUrl?: string;
  /** Pin this `eth_call` to a block height; omitted reads `latest`. See {@link roaxPublicClient}. */
  blockNumber?: bigint;
}): Promise<boolean> {
  return roaxPublicClient(args.rpcUrl, args.defaultRpcUrl).readContract({
    address: args.issuerAddr as Address,
    abi: DOGTAG_ISSUER_ABI,
    functionName: "isValid",
    args: [args.root as `0x${string}`],
    blockNumber: args.blockNumber,
  }) as Promise<boolean>;
}

/**
 * Reads DogTagIssuer.isRevoked(merkleRoot) - true once the root's revokedAt != 0. Distinguishes an
 * explicitly revoked credential from one that was simply never anchored (see {@link issuedAtOf}).
 */
export async function isRootRevoked(args: {
  issuerAddr: string;
  root: string;
  rpcUrl?: string;
  defaultRpcUrl?: string;
  /** Pin this `eth_call` to a block height; omitted reads `latest`. See {@link roaxPublicClient}. */
  blockNumber?: bigint;
}): Promise<boolean> {
  return roaxPublicClient(args.rpcUrl, args.defaultRpcUrl).readContract({
    address: args.issuerAddr as Address,
    abi: DOGTAG_ISSUER_ABI,
    functionName: "isRevoked",
    args: [args.root as `0x${string}`],
    blockNumber: args.blockNumber,
  }) as Promise<boolean>;
}

/**
 * Reads DogTagIssuer.issuedAt(merkleRoot) - the anchoring unix timestamp, or 0n if never issued.
 * `isValid == issuedAt != 0 && !isRevoked`; this getter is what lets the panel report `not_issued`
 * distinctly from `revoked`.
 */
export async function issuedAtOf(args: {
  issuerAddr: string;
  root: string;
  rpcUrl?: string;
  defaultRpcUrl?: string;
  /** Pin this `eth_call` to a block height; omitted reads `latest`. See {@link roaxPublicClient}. */
  blockNumber?: bigint;
}): Promise<bigint> {
  return roaxPublicClient(args.rpcUrl, args.defaultRpcUrl).readContract({
    address: args.issuerAddr as Address,
    abi: DOGTAG_ISSUER_ABI,
    functionName: "issuedAt",
    args: [args.root as `0x${string}`],
    blockNumber: args.blockNumber,
  }) as Promise<bigint>;
}

/**
 * Reads DogTagIssuer.issuedBy(merkleRoot) - the address that actually called `issue(root)` on this
 * clone (`issuedBy[r] = msg.sender`, H-1 originator binding), or the zero address for a root this
 * clone never issued.
 *
 * This is what makes the issuer-whitelist pillar self-resolving: the signer no longer has to be typed
 * in by an operator, so the pillar can be mandatory. `issue()` is `onlyWhitelisted`, so a genuinely
 * issued root's originator was whitelisted for that record type at issuance by construction.
 */
export async function issuedByOf(args: {
  issuerAddr: string;
  root: string;
  rpcUrl?: string;
  defaultRpcUrl?: string;
  /** Pin this `eth_call` to a block height; omitted reads `latest`. See {@link roaxPublicClient}. */
  blockNumber?: bigint;
}): Promise<string> {
  return roaxPublicClient(args.rpcUrl, args.defaultRpcUrl).readContract({
    address: args.issuerAddr as Address,
    abi: DOGTAG_ISSUER_ABI,
    functionName: "issuedBy",
    args: [args.root as `0x${string}`],
    blockNumber: args.blockNumber,
  }) as Promise<string>;
}

/**
 * Reads DogTagIssuerFactory.rootIssuer(merkleRoot) - the clone that actually issued this root, or the
 * zero address when no clone of this factory ever did.
 *
 * This is the anchor the whole issuer pillar hangs from. `registerRoot` is called only from inside a
 * clone's `issue()` and is `require(isClone[msg.sender])` + strictly write-once, so a contract the
 * factory never deployed can never appear here and a genuine root's issuer can never be overwritten.
 * Resolving the clone this way - rather than from the document's own `issuer.documentStore`, which
 * lives outside the Merkle root - is what stops a forger nominating a contract of their own to answer
 * every question about their forgery.
 */
export async function rootIssuerOf(args: {
  factoryAddr: string;
  root: string;
  rpcUrl?: string;
  defaultRpcUrl?: string;
  /** Pin this `eth_call` to a block height; omitted reads `latest`. See {@link roaxPublicClient}. */
  blockNumber?: bigint;
}): Promise<string> {
  return roaxPublicClient(args.rpcUrl, args.defaultRpcUrl).readContract({
    address: args.factoryAddr as Address,
    abi: DOGTAG_ISSUER_FACTORY_ABI,
    functionName: "rootIssuer",
    args: [args.root as `0x${string}`],
    blockNumber: args.blockNumber,
  }) as Promise<string>;
}

/**
 * Reads DogTagIssuer.recordType() - the clone's own immutable record type key. Read from the RESOLVED
 * clone so the whitelist question is asked about the record type the CHAIN says the root belongs to,
 * never the one the document's `issuer` block claims.
 */
export async function recordTypeOf(args: {
  issuerAddr: string;
  rpcUrl?: string;
  defaultRpcUrl?: string;
  /** Pin this `eth_call` to a block height; omitted reads `latest`. See {@link roaxPublicClient}. */
  blockNumber?: bigint;
}): Promise<string> {
  return roaxPublicClient(args.rpcUrl, args.defaultRpcUrl).readContract({
    address: args.issuerAddr as Address,
    abi: DOGTAG_ISSUER_ABI,
    functionName: "recordType",
    blockNumber: args.blockNumber,
  }) as Promise<string>;
}

/**
 * Reads DogTagIssuer.registry() - the registry that actually gates writes to this clone.
 *
 * Verification asks a registry of the CLIENT'S choosing whether the resolved signer is whitelisted.
 * That question is only meaningful if the registry asked is the one the clone's own `onlyWhitelisted`
 * consults: `_wl` is a plain per-contract mapping, so a DIFFERENT `IssuerRegistry` instance answers
 * about its own grants and knows nothing about this clone's. Comparing the two is the only way to tell
 * "this signer is authorised" from "some other registry happens to list this address".
 *
 * `registry` is set once in `initialize` from the factory's own `immutable registry` and has no setter,
 * so for a clone resolved through a matched factory/registry pair this always agrees - which is exactly
 * why a disagreement means the CONFIGURATION is wrong, not the credential.
 */
export async function issuerRegistryOf(args: {
  issuerAddr: string;
  rpcUrl?: string;
  defaultRpcUrl?: string;
  /** Pin this `eth_call` to a block height; omitted reads `latest`. See {@link roaxPublicClient}. */
  blockNumber?: bigint;
}): Promise<string> {
  return roaxPublicClient(args.rpcUrl, args.defaultRpcUrl).readContract({
    address: args.issuerAddr as Address,
    abi: DOGTAG_ISSUER_ABI,
    functionName: "registry",
    blockNumber: args.blockNumber,
  }) as Promise<string>;
}

/** A point in a log stream. Ordered by `(blockNumber, logIndex)`; `logIndex` is block-scoped and
 * therefore comparable ACROSS contracts within one block, which is what lets a registry grant and a
 * clone's issuance in the same block be sequenced against each other at all. */
export interface LogPoint {
  blockNumber: bigint;
  logIndex: number;
}

/** One `whitelistFor`/`delistFor` call, as observed in the log. */
export interface WhitelistGrantEvent extends LogPoint {
  kind: "whitelisted" | "delisted";
}

/**
 * The node returned a log carrying no `(blockNumber, logIndex)` - viem's shape for a log it considers
 * PENDING, where both fields are `null`.
 *
 * Its own answer, and neither of its neighbours: distinct from "the log said nothing" (an empty
 * history, a root with no anchoring event) and from "the read failed" (a rejected promise). A log
 * whose place in the sequence is unknown cannot be ordered, so the fold that consumes it cannot be
 * run - and both ways of running it anyway are wrong. Placing it at `(0n, 0)` puts a grant at the
 * very start of the chain, which sorts before every anchoring and can turn a delisted-before into an
 * authorised; dropping it removes an event that could have been the delisting, which does the same.
 * Either is could-not-check rendered as a verdict, which is exactly what this pillar refuses.
 *
 * Mirrors `log_point` (Rust, `stacks/{vet,government}/api/src/chain.rs`), `RoaxRpc.logPoint` (Kotlin)
 * and `RoaxRpc.logPoint` (Swift), each of which answers `Undetermined` for the same case.
 *
 * A SYMBOL rather than a string: a string sentinel is truthy AND carries a `.length`, so a consumer
 * written as `if (history.length)` would read it as a non-empty history. A symbol has no such member,
 * so every present and future consumer has to name the case for the module to typecheck.
 */
export const UNPOSITIONED_LOG: unique symbol = Symbol("dogtag.unpositionedLog");
export type UnpositionedLog = typeof UNPOSITIONED_LOG;

/** The ordering key of a mined log, or `null` when the node gave it no position. */
function logPoint(l: {
  blockNumber?: bigint | null;
  logIndex?: number | null;
}): LogPoint | null {
  return l.blockNumber == null || l.logIndex == null
    ? null
    : { blockNumber: l.blockNumber, logIndex: l.logIndex };
}

/**
 * The full grant/revocation history for one `(recordType, signer)` pair, oldest first.
 *
 * Read from LOGS rather than from `isWhitelistedFor`, because the getter answers only about NOW and
 * the question worth asking about an already-anchored credential is about THEN. `DogTagIssuer.sol:82`
 * states the rule the getter cannot express - "delisting is forward-only" - and `adminRevoke` exists
 * precisely because a delist does NOT retroactively invalidate what the signer already anchored.
 *
 * THREE outcomes, and no two of them may be folded together - that folding is the fail-open shape this
 * whole surface exists to refuse. An empty array is a real answer ("no grant was ever recorded for this
 * pair"); a read that FAILS throws; and a log the node gave no position resolves to
 * {@link UNPOSITIONED_LOG}, because a grant that cannot be sequenced cannot be folded and answering
 * anyway - at genesis, or by dropping it - could turn a delisted-before into an authorised.
 */
export async function whitelistGrantHistory(args: {
  registryAddr: string;
  /** The clone the grant is about. `IssuanceCapabilitySet` is indexed on it, not on a record type. */
  service: string;
  signer: string;
  rpcUrl?: string;
  defaultRpcUrl?: string;
  fromBlock?: bigint;
  /** Upper bound; omitted reads to `latest`. Pin it to the report's block for a reproducible answer. */
  toBlock?: bigint;
}): Promise<WhitelistGrantEvent[] | UnpositionedLog> {
  const client = roaxPublicClient(args.rpcUrl, args.defaultRpcUrl);
  const logs = await client.getLogs({
    address: args.registryAddr as Address,
    fromBlock: args.fromBlock ?? 0n,
    ...(args.toBlock === undefined ? {} : { toBlock: args.toBlock }),
    args: {
      service: args.service as Address,
      signer: args.signer as Address,
    },
    event: ISSUANCE_CAPABILITY_EVENT_ABI[0],
  });
  const events: WhitelistGrantEvent[] = [];
  for (const l of logs) {
    const at = logPoint(l);
    if (!at) return UNPOSITIONED_LOG;
    // A log whose `allowed` word did not decode is a malformed entry, not a fact about the
    // credential - it cannot be folded, so the whole answer is withheld.
    if (typeof l.args.allowed !== "boolean") return UNPOSITIONED_LOG;
    events.push({ kind: l.args.allowed ? "whitelisted" : "delisted", ...at });
  }
  return sortLogPoints(events);
}

/** Oldest first. Exported so the pure ordering rule has one definition and one set of tests. */
export function sortLogPoints<T extends LogPoint>(events: T[]): T[] {
  return [...events].sort((a, b) =>
    a.blockNumber === b.blockNumber
      ? a.logIndex - b.logIndex
      : a.blockNumber < b.blockNumber
        ? -1
        : 1,
  );
}

/** Total order over log points. `logIndex` is block-scoped, so it only breaks ties WITHIN a block. */
export function compareLogPoints(a: LogPoint, b: LogPoint): number {
  if (a.blockNumber !== b.blockNumber) return a.blockNumber < b.blockNumber ? -1 : 1;
  return a.logIndex - b.logIndex;
}

/**
 * Did the signer hold the capability AT THE MOMENT this root was anchored?
 *
 * `"authorized"` / `"notAuthorized"` are answers ABOUT the credential; `"undetermined"` says the
 * question could not be put. Only the first may contribute to a pass, only the second may refuse,
 * and the third must never be rendered as either.
 */
export type GrantAtIssuance = "authorized" | "notAuthorized" | "undetermined";

/**
 * Fold one `(recordType, signer)` grant history against the point a root was anchored.
 *
 * THE definition of the rule for the web, shared by the verifier and the bench so the surface that
 * decides and the surface that reports cannot drift. The state as of the anchoring point is the LAST
 * event at or before it.
 *
 * An EMPTY prior history is `"notAuthorized"`, not `"undetermined"`: the registry answered and its own
 * log records no grant, which is evidence about the credential rather than about our ability to check.
 * A log read that FAILED never reaches this function.
 *
 * THAT EMPTY-HISTORY RULE HOLDS ONLY FOR A GENERATION-1 AUTHORITY, and this function cannot tell:
 * `Whitelisted(bytes32 indexed recordType, address indexed signer)` puts the record-type key in
 * `topic1`, so its reader is a record-type caller in the sense `docs/CLIENT_REPOINT.md` means, via
 * logs rather than a getter. `ProviderRegistry` records grants as `IssuanceCapabilitySet(service,
 * signer, allowed)` — different name, different `topic0`, different shape — so the filter matches
 * NOTHING there and every genuine generation-2 credential would fold to a definite refusal. The
 * caller must therefore establish the authority's generation before treating an empty history as an
 * answer: see {@link authorityGenerationOf}.
 *
 * Mirrors `dogtag_standard::verify::grant_in_force_at` (Rust), `RoaxRpc.grantInForceAt` (Kotlin) and
 * `RoaxRpc.grantInForceAt` (Swift).
 */
export function grantInForceAt(
  history: readonly WhitelistGrantEvent[],
  anchoredAt: LogPoint,
): GrantAtIssuance {
  const prior = history.filter((e) => compareLogPoints(e, anchoredAt) <= 0);
  const asOf = sortLogPoints(prior)[prior.length - 1];
  return asOf?.kind === "whitelisted" ? "authorized" : "notAuthorized";
}





/**
 * Where this root was anchored, as a `(blockNumber, logIndex)` point - or `null` when this clone
 * emitted no `RootIssued` for it.
 *
 * `issuedAt` is a unix TIMESTAMP, which cannot be compared against a log's height without a
 * timestamp->block search. The anchoring event carries the height directly, so one filtered
 * `eth_getLogs` answers "when, in log order, was this anchored?" exactly and cheaply.
 *
 * A log the node gave no position resolves to {@link UNPOSITIONED_LOG} rather than being placed or
 * skipped, and that is NOT the same fact as `null`: "this contract emitted no anchoring event" is a
 * statement about the chain, while an unpositioned log is one we could not read. Skipping it would let
 * a later positioned sibling become the anchoring point, moving it FORWARD - and an anchoring moved
 * forward past a delisting is a false pass, the direction this pillar must never fail in.
 */
export async function rootIssuedAtLog(args: {
  issuerAddr: string;
  root: string;
  rpcUrl?: string;
  defaultRpcUrl?: string;
  fromBlock?: bigint;
  toBlock?: bigint;
}): Promise<LogPoint | null | UnpositionedLog> {
  const logs = await roaxPublicClient(args.rpcUrl, args.defaultRpcUrl).getLogs({
    address: args.issuerAddr as Address,
    event: ROOT_ISSUED_EVENT_ABI[0],
    args: { root: args.root as `0x${string}` },
    fromBlock: args.fromBlock ?? 0n,
    ...(args.toBlock === undefined ? {} : { toBlock: args.toBlock }),
  });
  // Write-once `issuedAt` makes a second RootIssued for one root impossible on an honest clone; take
  // the FIRST regardless, so a clone that somehow emitted twice cannot move the anchoring later.
  const points: LogPoint[] = [];
  for (const l of logs) {
    const at = logPoint(l);
    if (!at) return UNPOSITIONED_LOG;
    points.push(at);
  }
  return sortLogPoints(points)[0] ?? null;
}

