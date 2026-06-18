import {
  createPublicClient,
  http,
  keccak256,
  toBytes,
  type Abi,
  type Address,
  type PublicClient,
} from "viem";
import { roax } from "@dogtag/ui";
import { env } from "./env";

/** Matches the central backend's `record_type_key`: keccak256(recordType utf8 bytes). */
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

let cachedClient: PublicClient | null = null;
function client(): PublicClient {
  if (!cachedClient) {
    cachedClient = createPublicClient({
      chain: roax,
      transport: http(env.roaxRpc),
    });
  }
  return cachedClient;
}

export function whitelistConfigured(): boolean {
  return Boolean(env.issuerRegistryAddr);
}

/** Reads on-chain IssuerRegistry.isWhitelistedFor(keccak256(recordType), address). */
export async function isWhitelistedFor(recordType: string, address: string): Promise<boolean> {
  if (!whitelistConfigured()) {
    throw new Error("VITE_ISSUER_REGISTRY_ADDR not configured");
  }
  return client().readContract({
    address: env.issuerRegistryAddr as Address,
    abi: ISSUER_REGISTRY_ABI,
    functionName: "isWhitelistedFor",
    args: [recordTypeKey(recordType), address as Address],
  }) as Promise<boolean>;
}
