import { useCallback, useState } from "react";
import { useAccount, useSwitchChain } from "wagmi";
import {
  ROAX_CHAIN_ID,
  roax,
  roaxAddChainParams,
} from "./chain";
import {
  isUnrecognizedChain,
  isUserRejection,
  walletErrorMessage,
} from "./walletError";

type Eip1193Provider = {
  request: (args: { method: string; params?: unknown[] }) => Promise<unknown>;
};

export interface UseRoaxChainResult {
  /** true when the connected wallet is on ROAX (chainId 135) */
  isOnRoax: boolean;
  switching: boolean;
  error: string | null;
  /** Switch the CONNECTED wallet to ROAX, adding the chain first if it does not know it. */
  switchToRoax: () => Promise<boolean>;
}

/**
 * Switch/add the ROAX chain in the connected wallet (impl §3.8 chain-add calldata).
 *
 * **THIS SENDS TO THE CONNECTED CONNECTOR, NEVER TO `window.ethereum`.** It used to read the global,
 * and that is a real defect rather than a stylistic one: connection goes through AppKit/wagmi, which
 * uses EIP-6963 discovery and connects to the wallet the USER PICKED, while `window.ethereum` is
 * whichever extension won a race to claim the global. With two extensions installed those are
 * routinely different wallets. Found live - a captain connected one wallet and the switch request
 * opened in another, which was sitting on Ethereum Mainnet, so its dialog reported signing against
 * chain `0x1` while the site had asked for `0x87`. Every portal mounts `WalletButton`, so this
 * reached all of them.
 *
 * `useSwitchChain` is the primitive because it routes to the connected connector by construction and
 * performs the add itself for connectors that support it; there is no global to get wrong.
 *
 * **The add fallback is NOT keyed on code 4902 alone.** That is MetaMask's spelling of "I do not
 * know this chain", and a wallet answering in its own dialect used to fall straight through to a
 * dead end - the captain got `4100` and had to add ROAX by hand. {@link isUnrecognizedChain} reads
 * the whole wrapped error chain and the message text; see `walletError.ts` for why both.
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
  const { chainId, connector } = useAccount();
  const { switchChainAsync } = useSwitchChain();
  const [switching, setSwitching] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const switchToRoax = useCallback(async (): Promise<boolean> => {
    setSwitching(true);
    setError(null);
    try {
      // `chainId` minus the hex `chainId` field: wagmi supplies that itself, and passing the whole
      // EIP-3085 object would give it two sources for one value.
      const { chainId: _hex, ...addParams } = roaxAddChainParams(defaultRpcUrl);
      await switchChainAsync({
        chainId: ROAX_CHAIN_ID,
        addEthereumChainParameter: {
          ...addParams,
          // The source object is `as const`, and wagmi wants mutable arrays. Copied rather than
          // cast, so the shared constant cannot be reached through this reference.
          rpcUrls: [...addParams.rpcUrls],
          blockExplorerUrls: [...addParams.blockExplorerUrls],
          nativeCurrency: { ...addParams.nativeCurrency },
        },
      });
      return true;
    } catch (err) {
      // A refusal is the person's decision, not a fault, and must never be retried by adding the
      // chain underneath them.
      if (isUserRejection(err)) {
        setError("You declined the network switch in your wallet.");
        return false;
      }
      if (isUnrecognizedChain(err)) {
        // The explicit add, through the CONNECTED connector's own provider. Reached only when
        // wagmi's own add attempt did not fire or did not take - which is exactly the case a
        // wallet spelling "unknown chain" in a non-4902 dialect produces.
        try {
          const provider = (await connector?.getProvider()) as Eip1193Provider | undefined;
          if (!provider?.request) throw new Error("the connected wallet exposed no provider");
          await provider.request({
            method: "wallet_addEthereumChain",
            params: [roaxAddChainParams(defaultRpcUrl)],
          });
          return true;
        } catch (addErr) {
          if (isUserRejection(addErr)) {
            setError("You declined adding the ROAX network in your wallet.");
            return false;
          }
          setError(`Could not add the ROAX network: ${walletErrorMessage(addErr)}`);
          return false;
        }
      }
      setError(`Could not switch to ROAX: ${walletErrorMessage(err)}`);
      return false;
    } finally {
      setSwitching(false);
    }
  }, [switchChainAsync, connector, defaultRpcUrl]);

  return {
    isOnRoax: chainId === ROAX_CHAIN_ID,
    switching,
    error,
    switchToRoax,
  };
}
