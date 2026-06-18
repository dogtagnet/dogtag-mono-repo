import { useAppKit } from "@reown/appkit/react";
import { Wallet } from "lucide-react";
import { useAccount, useDisconnect } from "wagmi";
import { Badge } from "../components/Badge";
import { Button } from "../components/Button";
import { useRoaxChain } from "./useRoaxChain";

export function shortAddress(addr?: string): string {
  if (!addr) return "";
  return `${addr.slice(0, 6)}…${addr.slice(-4)}`;
}

/**
 * Connect MetaMask / WalletConnect via Reown AppKit, then surface ROAX-chain state
 * with a "Switch to ROAX" action (impl §5.0). Requires WagmiProvider + AppKit init
 * (createWalletConfig) above it.
 */
export function WalletButton({ className }: { className?: string }) {
  const { open } = useAppKit();
  const { address, isConnected } = useAccount();
  const { disconnect } = useDisconnect();
  const { isOnRoax, switchToRoax, switching } = useRoaxChain();

  if (!isConnected) {
    return (
      <Button variant="outline" className={className} onClick={() => open()}>
        <Wallet className="h-4 w-4" />
        Connect wallet
      </Button>
    );
  }

  return (
    <div className="flex items-center gap-2">
      {isOnRoax ? (
        <Badge variant="success">ROAX</Badge>
      ) : (
        <Button variant="outline" size="sm" loading={switching} onClick={() => void switchToRoax()}>
          Switch to ROAX
        </Button>
      )}
      <Button variant="ghost" size="sm" className={className} onClick={() => open()}>
        <Wallet className="h-4 w-4" />
        {shortAddress(address)}
      </Button>
      <Button variant="ghost" size="sm" onClick={() => disconnect()}>
        Disconnect
      </Button>
    </div>
  );
}
