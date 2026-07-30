import { useCallback, useState } from "react";
import { useAccount } from "wagmi";
import {
  ROAX_CHAIN_ID,
  ROAX_CHAIN_ID_HEX,
  roax,
  roaxAddChainParams,
} from "./chain";

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
 *
 * The add metadata always names the app's BUNDLED endpoint, never the browser's DogTag endpoint
 * choice. That choice is scoped to DogTag's own direct reads, where the chain guard re-runs before
 * every request; writing it into the wallet's persistent chain configuration would hand a
 * user-typed peer the wallet's own traffic — transaction broadcast included — under a guard that
 * could only ever have run once, at add time. Repointing a wallet is a separate, explicit action.
 * Nothing here may read the endpoint preference: this module deliberately does not import it, so
 * the bundled endpoint is the only URL in scope to hand to the wallet.
 */
export function useRoaxChain(defaultRpcUrl: string = roax.rpcUrls.default.http[0]): UseRoaxChainResult {
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
            params: [roaxAddChainParams(defaultRpcUrl)],
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
  }, [defaultRpcUrl]);

  return {
    isOnRoax: chainId === ROAX_CHAIN_ID,
    switching,
    error,
    switchToRoax,
  };
}
