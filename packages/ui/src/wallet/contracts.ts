import {
  createPublicClient,
  keccak256,
  toBytes,
  type Abi,
  type Address,
  type PublicClient,
} from "viem";
import { guardedRoaxTransport } from "../chain/rpcEndpoint";
import { roax } from "./chain";

/**
 * Deployed ROAX contract addresses (contracts/deployments/roax.json). Exposed as defaults; each
 * portal may override via `VITE_*` env. These are the addresses the whitelist viewer + the
 * issue-status on-chain poller read against.
 *
 * CUTOVER (registry plan S-13/S-14, `docs/CLIENT_REPOINT.md`): THREE of these are moving addresses,
 * and only TWO may actually be repointed at C-9.
 *
 * `DogTagIssuerFactory` → **`CloneProvenanceRouter`**: this module READS `rootIssuer`, so it takes
 * the router, never `DogTagIssuerFactoryV2` - that would resolve every historical root to
 * `address(0)` and surface as an indeterminate issuer-whitelist pillar rather than an error.
 * `VerificationRegistryConsent` → its V2.
 *
 * `IssuerRegistry` is **BLOCKED**, and being read-only does not rescue it. `ProviderRegistry`
 * implements `isWhitelistedFor` but branches on `msg.sender`, and `readContract` below passes no
 * `account` - so `msg.sender` is `0x0` and it answers the orthogonal VERIFY-key capability instead
 * of the issuance whitelist. `isWhitelistedFor` here asks with `recordTypeKey(...)`, which is never
 * a VERIFY key, so every answer would be a confident `false` for a genuine issuer signer. The
 * unblock is the `isRecognizedIssuer(service, signer)` migration (`docs/ISSUER_V2_OWNERSHIP.md` §8).
 * The governing condition it violates: an address may be repointed only when the successor answers
 * THE SAME QUESTION FOR THE SAME INPUTS - a matching selector is not evidence of that.
 *
 * `DogTagSBT` here is already the reused `DogTagSBTConsent` and must NOT move - it is listed with
 * the movers because it looks like it should move, not because it does. Because these are DEFAULTS,
 * an unset `VITE_*` override after the cutover silently keeps reading generation 1 - so move the
 * constants, do not rely on the env. `make check-cutover-consumers` is the gate, and it is what
 * establishes the counts above rather than a reading of this list.
 */
export const DEPLOYED_ADDRESSES = {
  IssuerRegistry: "0xAEE540350292E49A9AeDf19Dd4C3BAc6ABeE6c21",
  DogTagSBT: "0xBEbc45A838643D27004827b797b30A464b2b02c0",
  VerificationRegistry: "0x4E2f0996e1CB4E24F1053346f3da2186906835E8",
  /**
   * The OWNER-HIDDEN verification registry — the live one, and a DIFFERENT contract from
   * `VerificationRegistry` above (which it supersedes). Its `Verified(uint256 indexed dogTagId, ...)`
   * is the only event that indexes a tag id, so it is what on-chain tag discovery must read; pointing
   * a scan at the superseded address returns zero events and is indistinguishable from a tag with no
   * history. Matches `VERIFICATION_REGISTRY_CONSENT_ADDR` in the deployed stacks.
   */
  VerificationRegistryConsent: "0xaBFd6f6E31780EBcB7ABd28A2a9bCfc9C8e6A77B",
  Poseidon6: "0x58091F2320c78ed6c6D1C02CB7E5c7578f1349db",
  ConsentKeyRegistry: "0xA74DDe4a9b5b5b9045D9244907dE5d84C75BD671",
  DogTagIssuerFactory: "0xED20269E3eBF0119739aaB5258741F3aEb49F140",
  Groth16Verifier: "0xEEFCfAF026931b7325472A88fd14Ee780Da13559",
  admin: "0x119F8c7F6D7EC10E7376983739C6f46cF9CC3E96",
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
 * `IssuerRegistry`'s two grant-lifecycle events, both indexed on exactly the pair a whitelist question
 * is asked about - so the full grant history for one `(recordType, signer)` is one filtered
 * `eth_getLogs`, with no scan and no per-log decode of irrelevant entries.
 */
const ISSUER_REGISTRY_EVENT_ABI = [
  {
    type: "event",
    name: "Whitelisted",
    inputs: [
      { name: "recordType", type: "bytes32", indexed: true },
      { name: "signer", type: "address", indexed: true },
    ],
  },
  {
    type: "event",
    name: "Delisted",
    inputs: [
      { name: "recordType", type: "bytes32", indexed: true },
      { name: "signer", type: "address", indexed: true },
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

const ISSUER_DOMAIN_REGISTRY_ABI = [
  {
    // `getBinding(clone)` — the published issuer->domain claim. Deliberately does NOT revert on an
    // unknown clone: "no domain claimed" is a normal day-one state, signalled by `updatedAt == 0`.
    type: "function",
    name: "getBinding",
    stateMutability: "view",
    inputs: [{ name: "clone", type: "address" }],
    outputs: [
      {
        name: "",
        type: "tuple",
        components: [
          { name: "domain", type: "string" },
          { name: "updatedAt", type: "uint64" },
          { name: "updatedAtBlock", type: "uint64" },
          { name: "setBy", type: "address" },
        ],
      },
    ],
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
 * The full grant/revocation history for one `(recordType, signer)` pair, oldest first.
 *
 * Read from LOGS rather than from `isWhitelistedFor`, because the getter answers only about NOW and
 * the question worth asking about an already-anchored credential is about THEN. `DogTagIssuer.sol:82`
 * states the rule the getter cannot express - "delisting is forward-only" - and `adminRevoke` exists
 * precisely because a delist does NOT retroactively invalidate what the signer already anchored.
 *
 * An empty array is a real answer ("no grant was ever recorded for this pair"); a read that FAILS
 * throws, so a caller can keep it apart from "the log could not be reached". Folding those two is the
 * fail-open shape this whole surface exists to refuse.
 */
export async function whitelistGrantHistory(args: {
  registryAddr: string;
  recordTypeKey: string;
  signer: string;
  rpcUrl?: string;
  defaultRpcUrl?: string;
  fromBlock?: bigint;
  /** Upper bound; omitted reads to `latest`. Pin it to the report's block for a reproducible answer. */
  toBlock?: bigint;
}): Promise<WhitelistGrantEvent[]> {
  const client = roaxPublicClient(args.rpcUrl, args.defaultRpcUrl);
  const range = {
    address: args.registryAddr as Address,
    fromBlock: args.fromBlock ?? 0n,
    ...(args.toBlock === undefined ? {} : { toBlock: args.toBlock }),
    args: {
      recordType: args.recordTypeKey as `0x${string}`,
      signer: args.signer as Address,
    },
  } as const;
  const [granted, revoked] = await Promise.all([
    client.getLogs({ ...range, event: ISSUER_REGISTRY_EVENT_ABI[0] }),
    client.getLogs({ ...range, event: ISSUER_REGISTRY_EVENT_ABI[1] }),
  ]);
  const events: WhitelistGrantEvent[] = [
    ...granted.map((l) => ({
      kind: "whitelisted" as const,
      blockNumber: l.blockNumber ?? 0n,
      logIndex: l.logIndex ?? 0,
    })),
    ...revoked.map((l) => ({
      kind: "delisted" as const,
      blockNumber: l.blockNumber ?? 0n,
      logIndex: l.logIndex ?? 0,
    })),
  ];
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
 * Mirrors `dogtag_standard::verify::grant_in_force_at` (Rust), `RoaxRpc.grantInForceAt` (Kotlin) and
 * `Net.grantInForceAt` (Swift).
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
 */
export async function rootIssuedAtLog(args: {
  issuerAddr: string;
  root: string;
  rpcUrl?: string;
  defaultRpcUrl?: string;
  fromBlock?: bigint;
  toBlock?: bigint;
}): Promise<LogPoint | null> {
  const logs = await roaxPublicClient(args.rpcUrl, args.defaultRpcUrl).getLogs({
    address: args.issuerAddr as Address,
    event: ROOT_ISSUED_EVENT_ABI[0],
    args: { root: args.root as `0x${string}` },
    fromBlock: args.fromBlock ?? 0n,
    ...(args.toBlock === undefined ? {} : { toBlock: args.toBlock }),
  });
  // Write-once `issuedAt` makes a second RootIssued for one root impossible on an honest clone; take
  // the FIRST regardless, so a clone that somehow emitted twice cannot move the anchoring later.
  const first = sortLogPoints(
    logs.map((l) => ({ blockNumber: l.blockNumber ?? 0n, logIndex: l.logIndex ?? 0 })),
  )[0];
  return first ?? null;
}

/**
 * The on-chain issuer->domain claim published for `cloneAddr`, or `null` when none is.
 *
 * Read from the DOMAIN REGISTRY, never from the credential: `issuer.domain` sits outside the Merkle
 * root, so the document's own claim is exactly the field a relabelling attack rewrites. This getter is
 * the unforgeable half of that comparison.
 *
 * `null` is the honest "this issuer has published no domain claim" - the normal day-one state, and NOT
 * an error. A read that FAILS throws, so a caller can keep "no claim" and "could not ask" apart; folding
 * the two is the fail-open bug the six-state `IssuerDomainBinding` model exists to prevent.
 *
 * The registry address is a PARAMETER with no default, unlike the factory/registry addresses above. The
 * contract set is still being revised and `IssuerDomainRegistry` may yet be folded elsewhere, so a
 * caller with nothing configured must report the check as unavailable rather than read a stale constant.
 */
export async function issuerDomainClaimOf(args: {
  domainRegistryAddr: string;
  cloneAddr: string;
  rpcUrl?: string;
  defaultRpcUrl?: string;
  /** Pin this `eth_call` to a block height; omitted reads `latest`. See {@link roaxPublicClient}. */
  blockNumber?: bigint;
}): Promise<{ domain: string; updatedAt: bigint; updatedAtBlock: bigint; setBy: string } | null> {
  const b = (await roaxPublicClient(args.rpcUrl, args.defaultRpcUrl).readContract({
    address: args.domainRegistryAddr as Address,
    abi: ISSUER_DOMAIN_REGISTRY_ABI,
    functionName: "getBinding",
    args: [args.cloneAddr as Address],
    blockNumber: args.blockNumber,
  })) as { domain: string; updatedAt: bigint; updatedAtBlock: bigint; setBy: string };
  // `updatedAt == 0` is the contract's own "no binding published" sentinel, not a zero timestamp.
  return b.updatedAt === 0n ? null : b;
}
