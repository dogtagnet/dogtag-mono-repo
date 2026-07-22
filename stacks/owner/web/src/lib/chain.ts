// Read-only ROAX access for the owner wallet. All reads are gasless (the owner never pays gas — the
// whole design keeps the holder gasless). DogTagIssuer.isValid(root) supplies the on-chain
// "issuance" pillar for a held credential.
import { createPublicClient, defineChain, http } from "viem";
import { ROAX_CHAIN_ID, ROAX_RPC_URL } from "./config";

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

/** DogTagIssuer clone (== issuer.documentStore) isValid(root) — true once the root is anchored. */
export async function isRootAnchored(documentStore: string, merkleRoot: string): Promise<boolean> {
  return publicClient.readContract({
    address: documentStore as `0x${string}`,
    abi: ISSUER_ABI,
    functionName: "isValid",
    args: [merkleRoot as `0x${string}`],
  });
}
