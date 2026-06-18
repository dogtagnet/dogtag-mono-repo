import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { useMemo, type ReactNode } from "react";
import { WagmiProvider } from "wagmi";
import { createWalletConfig, type WalletConfigOptions } from "./config";

/**
 * One-shot provider: initializes Reown AppKit + wagmi and wraps children in
 * WagmiProvider + a react-query client (wagmi v2 needs both). Mount once at app root.
 */
export function WalletProvider({
  children,
  options,
}: {
  children: ReactNode;
  options?: WalletConfigOptions;
}) {
  const { wagmiConfig, queryClient } = useMemo(() => {
    const cfg = createWalletConfig(options);
    return { wagmiConfig: cfg.wagmiConfig, queryClient: new QueryClient() };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  return (
    <WagmiProvider config={wagmiConfig}>
      <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
    </WagmiProvider>
  );
}
