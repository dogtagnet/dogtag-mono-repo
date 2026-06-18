import { WagmiAdapter } from "@reown/appkit-adapter-wagmi";
import type { AppKitNetwork } from "@reown/appkit/networks";
import { createAppKit } from "@reown/appkit/react";
import { roax } from "./chain";

/**
 * Reown AppKit + wagmi v2 setup (impl §5.0). The projectId is read from
 * `import.meta.env.VITE_REOWN_PROJECT_ID`; a placeholder default keeps the build green.
 * Replace with a real Reown Cloud projectId before shipping (WalletConnect transport
 * will not function with the placeholder).
 */
const PLACEHOLDER_PROJECT_ID = "REPLACE_WITH_REOWN_PROJECT_ID";

export interface WalletConfigOptions {
  projectId?: string;
  appName?: string;
  appDescription?: string;
  appUrl?: string;
}

export const roaxNetwork = roax as unknown as AppKitNetwork;

export function createWalletConfig(opts: WalletConfigOptions = {}) {
  const projectId =
    opts.projectId ||
    (import.meta as unknown as { env?: Record<string, string> }).env?.VITE_REOWN_PROJECT_ID ||
    PLACEHOLDER_PROJECT_ID;

  const networks: [AppKitNetwork, ...AppKitNetwork[]] = [roaxNetwork];

  const wagmiAdapter = new WagmiAdapter({
    networks,
    projectId,
    ssr: false,
  });

  createAppKit({
    adapters: [wagmiAdapter],
    networks,
    projectId,
    metadata: {
      name: opts.appName ?? "DogTag Portal",
      description: opts.appDescription ?? "DogTag credentialing portal",
      url: opts.appUrl ?? (typeof window !== "undefined" ? window.location.origin : "https://dogtag.io"),
      icons: [],
    },
    features: { analytics: false, email: false, socials: false },
    defaultNetwork: roaxNetwork,
  });

  return { wagmiConfig: wagmiAdapter.wagmiConfig, projectId, isPlaceholder: projectId === PLACEHOLDER_PROJECT_ID };
}
