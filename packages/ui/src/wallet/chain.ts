import { defineChain } from "viem";

/**
 * ROAX — the PLASMA chain DogTag deploys to (impl §3.8).
 * chainId 135 (0x87), native PLASMA (18 decimals).
 */
export const ROAX_CHAIN_ID = 135;
export const ROAX_CHAIN_ID_HEX = "0x87";

export const roax = defineChain({
  id: ROAX_CHAIN_ID,
  name: "ROAX",
  nativeCurrency: { name: "Plasma", symbol: "PLASMA", decimals: 18 },
  rpcUrls: {
    default: { http: ["https://devrpc.roax.net"] },
    public: { http: ["https://devrpc.roax.net"] },
  },
  blockExplorers: {
    default: { name: "ROAX Explorer", url: "https://explorer.roax.net" },
  },
  testnet: true,
});

/** Params for wallet_addEthereumChain (EIP-3085) — used on the 4902 fallback. */
export const ROAX_ADD_CHAIN_PARAMS = {
  chainId: ROAX_CHAIN_ID_HEX,
  chainName: "ROAX",
  nativeCurrency: { name: "Plasma", symbol: "PLASMA", decimals: 18 },
  rpcUrls: ["https://devrpc.roax.net"],
  blockExplorerUrls: ["https://explorer.roax.net"],
} as const;

export function explorerTxUrl(txHash: string): string {
  return `${roax.blockExplorers.default.url}/tx/${txHash}`;
}

export function explorerAddressUrl(addr: string): string {
  return `${roax.blockExplorers.default.url}/address/${addr}`;
}
