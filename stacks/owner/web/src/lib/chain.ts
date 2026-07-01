// Read-only ROAX access for the owner wallet. All reads are gasless (the owner never pays gas — the
// whole design keeps the holder gasless). Two reads are used:
//   - DogTagIssuer.isValid(root)     -> the on-chain "issuance" pillar for a held credential
//   - ConsentKeyRegistry.bindNonce   -> the per-wallet replay nonce for the EIP-712 consent-key bind
import { createPublicClient, defineChain, http } from "viem";
import { CONTRACTS, ROAX_CHAIN_ID, ROAX_RPC_URL } from "./config";

export const roax = defineChain({
  id: ROAX_CHAIN_ID,
  name: "ROAX",
  nativeCurrency: { name: "Plasma", symbol: "PLASMA", decimals: 18 },
  rpcUrls: { default: { http: [ROAX_RPC_URL] } },
});

const publicClient = createPublicClient({ chain: roax, transport: http(ROAX_RPC_URL) });

const ISSUER_ABI = [
  {
    type: "function",
    name: "isValid",
    stateMutability: "view",
    inputs: [{ name: "r", type: "bytes32" }],
    outputs: [{ name: "", type: "bool" }],
  },
] as const;

const CONSENT_KEY_ABI = [
  {
    type: "function",
    name: "bindNonce",
    stateMutability: "view",
    inputs: [{ name: "wallet", type: "address" }],
    outputs: [{ name: "", type: "uint256" }],
  },
] as const;

/** DogTagIssuer clone (== issuer.documentStore) isValid(root) — true once the root is anchored. */
export async function isRootAnchored(documentStore: string, merkleRoot: string): Promise<boolean> {
  return publicClient.readContract({
    address: documentStore as `0x${string}`,
    abi: ISSUER_ABI,
    functionName: "isValid",
    args: [merkleRoot as `0x${string}`],
  });
}

/** The wallet's current consent-key bind nonce (0 when never bound). Best-effort — returns 0 on RPC error. */
export async function readBindNonce(wallet: `0x${string}`): Promise<bigint> {
  try {
    return await publicClient.readContract({
      address: CONTRACTS.ConsentKeyRegistry as `0x${string}`,
      abi: CONSENT_KEY_ABI,
      functionName: "bindNonce",
      args: [wallet],
    });
  } catch {
    return 0n;
  }
}
