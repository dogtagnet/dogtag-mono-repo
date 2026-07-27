import {
  createPublicClient,
  http,
  keccak256,
  toBytes,
  type Abi,
  type Address,
  type PublicClient,
} from "viem";
import { roax } from "./chain";

/**
 * Deployed ROAX contract addresses (contracts/deployments/roax.json). Exposed as defaults; each
 * portal may override via `VITE_*` env. These are the addresses the whitelist viewer + the
 * issue-status on-chain poller read against.
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

const clientCache = new Map<string, PublicClient>();

/** A cached viem public client for ROAX over the given RPC (defaults to the ROAX devrpc). */
export function roaxPublicClient(rpcUrl?: string): PublicClient {
  const url = rpcUrl ?? roax.rpcUrls.default.http[0];
  let c = clientCache.get(url);
  if (!c) {
    c = createPublicClient({ chain: roax, transport: http(url) });
    clientCache.set(url, c);
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
  return roaxPublicClient(args.rpcUrl).readContract({
    address: args.registryAddr as Address,
    abi: ISSUER_REGISTRY_ABI,
    functionName: "isWhitelistedFor",
    args: [key as `0x${string}`, args.address as Address],
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
}): Promise<boolean> {
  return roaxPublicClient(args.rpcUrl).readContract({
    address: args.issuerAddr as Address,
    abi: DOGTAG_ISSUER_ABI,
    functionName: "isValid",
    args: [args.root as `0x${string}`],
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
}): Promise<boolean> {
  return roaxPublicClient(args.rpcUrl).readContract({
    address: args.issuerAddr as Address,
    abi: DOGTAG_ISSUER_ABI,
    functionName: "isRevoked",
    args: [args.root as `0x${string}`],
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
}): Promise<bigint> {
  return roaxPublicClient(args.rpcUrl).readContract({
    address: args.issuerAddr as Address,
    abi: DOGTAG_ISSUER_ABI,
    functionName: "issuedAt",
    args: [args.root as `0x${string}`],
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
}): Promise<string> {
  return roaxPublicClient(args.rpcUrl).readContract({
    address: args.issuerAddr as Address,
    abi: DOGTAG_ISSUER_ABI,
    functionName: "issuedBy",
    args: [args.root as `0x${string}`],
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
}): Promise<string> {
  return roaxPublicClient(args.rpcUrl).readContract({
    address: args.factoryAddr as Address,
    abi: DOGTAG_ISSUER_FACTORY_ABI,
    functionName: "rootIssuer",
    args: [args.root as `0x${string}`],
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
}): Promise<string> {
  return roaxPublicClient(args.rpcUrl).readContract({
    address: args.issuerAddr as Address,
    abi: DOGTAG_ISSUER_ABI,
    functionName: "recordType",
  }) as Promise<string>;
}
