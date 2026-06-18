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
  IssuerRegistry: "0x5d86e4CF98A34Ae0576F190F8d209c2943a9C79c",
  DogTagSBT: "0x1FB8986573Ac36d532cF7d5a5352202B094D4233",
  VerificationRegistry: "0x19C1B5f80c41EE864149500bdF998Dd18aec2a43",
  Poseidon6: "0x58091F2320c78ed6c6D1C02CB7E5c7578f1349db",
  ConsentKeyRegistry: "0xFD277b9B33a4b299fe0b08dfA19eA0372b70745b",
  DogTagIssuerFactory: "0xd3179AbBfb0274D0a5F7017d76015A93C159511D",
  Groth16Verifier: "0x138b433071Ad806E841B5AD53623290a9bf21761",
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
] as const satisfies Abi;

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

/** Reads IssuerRegistry.isWhitelistedFor(keccak256(recordType), address). */
export async function isWhitelistedFor(args: {
  registryAddr: string;
  recordType: string;
  address: string;
  rpcUrl?: string;
}): Promise<boolean> {
  return roaxPublicClient(args.rpcUrl).readContract({
    address: args.registryAddr as Address,
    abi: ISSUER_REGISTRY_ABI,
    functionName: "isWhitelistedFor",
    args: [recordTypeKey(args.recordType), args.address as Address],
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
