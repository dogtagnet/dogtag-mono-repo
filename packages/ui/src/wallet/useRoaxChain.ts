import { useCallback, useState } from "react";
import { useAccount } from "wagmi";
import { ROAX_ADD_CHAIN_PARAMS, ROAX_CHAIN_ID, ROAX_CHAIN_ID_HEX } from "./chain";

type Eip1193Provider = {
  request: (args: { method: string; params?: unknown[] }) => Promise<unknown>;
};

function getInjected(): Eip1193Provider | undefined {
  if (typeof window === "undefined") return undefined;
  return (window as unknown as { ethereum?: Eip1193Provider }).ethereum;
}

export interface UseRoaxChainResult {
  /** true when the connected wallet is on ROAX (chainId 135) */
  isOnRoax: boolean;
  switching: boolean;
  error: string | null;
  /** wallet_switchEthereumChain(0x87) → on 4902, wallet_addEthereumChain(ROAX) */
  switchToRoax: () => Promise<boolean>;
}

/**
 * Switch/add the ROAX chain in the connected wallet (impl §3.8 chain-add calldata).
 * Falls back to wallet_addEthereumChain on error code 4902 (unrecognized chain).
 */
export function useRoaxChain(): UseRoaxChainResult {
  const { chainId } = useAccount();
  const [switching, setSwitching] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const switchToRoax = useCallback(async (): Promise<boolean> => {
    const provider = getInjected();
    if (!provider) {
      setError("No injected wallet provider found");
      return false;
    }
    setSwitching(true);
    setError(null);
    try {
      await provider.request({
        method: "wallet_switchEthereumChain",
        params: [{ chainId: ROAX_CHAIN_ID_HEX }],
      });
      return true;
    } catch (err) {
      const code = (err as { code?: number })?.code;
      // 4902 = chain not added to the wallet yet → add it, then it's selected.
      if (code === 4902) {
        try {
          await provider.request({
            method: "wallet_addEthereumChain",
            params: [ROAX_ADD_CHAIN_PARAMS],
          });
          return true;
        } catch (addErr) {
          setError((addErr as Error)?.message ?? "Failed to add ROAX chain");
          return false;
        }
      }
      setError((err as Error)?.message ?? "Failed to switch to ROAX");
      return false;
    } finally {
      setSwitching(false);
    }
  }, []);

  return {
    isOnRoax: chainId === ROAX_CHAIN_ID,
    switching,
    error,
    switchToRoax,
  };
}
