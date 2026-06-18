import { AlertTriangle, CheckCircle2, XCircle } from "lucide-react";
import { formatUnits } from "viem";
import { useAccount, useBalance } from "wagmi";
import { Badge } from "../components/Badge";
import { Card, CardContent, CardHeader, CardTitle } from "../components/Card";
import type { SigningMode, WhitelistRow } from "../api/types";
import { explorerAddressUrl } from "../wallet/chain";
import { useRoaxChain } from "../wallet/useRoaxChain";
import { shortAddress } from "../wallet/WalletButton";

type GenesisState = "initialized" | "locked" | "uninitialized" | "unknown";

export interface StatusPanelProps {
  mode: SigningMode;
  /** per-(address × recordType) whitelist matrix from GET /issuer/signers */
  whitelist?: WhitelistRow[];
  // backend-mode inputs (the wallet hooks supply wallet-mode inputs directly)
  genesisState?: GenesisState;
  backendSignerAddress?: string;
  /** PLASMA balance string for the backend signer (gas-funding health) */
  backendPlasmaBalance?: string;
}

function Row({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="flex items-center justify-between gap-3 py-1.5">
      <span className="text-sm text-muted">{label}</span>
      <span className="text-sm font-medium text-onSurface">{children}</span>
    </div>
  );
}

/**
 * Signing-context status (impl §5.0).
 *  - wallet mode → connected address + ROAX-chain check + per-recordType whitelist badges.
 *  - backend mode → genesis state + backend signer address + PLASMA balance (gas health).
 */
export function StatusPanel({
  mode,
  whitelist,
  genesisState,
  backendSignerAddress,
  backendPlasmaBalance,
}: StatusPanelProps) {
  const { address, isConnected } = useAccount();
  const { isOnRoax } = useRoaxChain();
  const { data: walletBalance } = useBalance({
    address,
    query: { enabled: mode === "wallet" && Boolean(address) },
  });

  return (
    <Card>
      <CardHeader>
        <CardTitle className="text-base">Signing status</CardTitle>
      </CardHeader>
      <CardContent className="divide-y divide-border">
        {mode === "wallet" ? (
          <>
            <Row label="Connected wallet">
              {isConnected ? (
                <a
                  href={address ? explorerAddressUrl(address) : "#"}
                  target="_blank"
                  rel="noreferrer"
                  className="font-mono text-primary hover:underline"
                >
                  {shortAddress(address)}
                </a>
              ) : (
                <Badge variant="warning">Not connected</Badge>
              )}
            </Row>
            <Row label="Network">
              {isOnRoax ? (
                <Badge variant="success">
                  <CheckCircle2 className="h-3 w-3" /> ROAX
                </Badge>
              ) : (
                <Badge variant="danger">
                  <AlertTriangle className="h-3 w-3" /> Wrong network
                </Badge>
              )}
            </Row>
            <Row label="PLASMA balance">
              {walletBalance ? `${Number(walletBalance.formatted).toFixed(4)} PLASMA` : "—"}
            </Row>
            <div className="py-2">
              <span className="text-sm text-muted">Whitelist (per record type)</span>
              <div className="mt-2 flex flex-wrap gap-1.5">
                {whitelist && whitelist.length > 0 ? (
                  whitelist.map((w) => (
                    <Badge key={w.recordType} variant={w.whitelisted ? "success" : "neutral"}>
                      {w.whitelisted ? (
                        <CheckCircle2 className="h-3 w-3" />
                      ) : (
                        <XCircle className="h-3 w-3" />
                      )}
                      {w.recordType}
                    </Badge>
                  ))
                ) : (
                  <span className="text-sm text-muted">No record types configured</span>
                )}
              </div>
            </div>
          </>
        ) : (
          <>
            <Row label="Genesis state">
              <Badge variant={genesisState === "initialized" ? "success" : "warning"}>
                {(genesisState ?? "unknown").toUpperCase()}
              </Badge>
            </Row>
            <Row label="Backend signer">
              {backendSignerAddress ? (
                <a
                  href={explorerAddressUrl(backendSignerAddress)}
                  target="_blank"
                  rel="noreferrer"
                  className="font-mono text-primary hover:underline"
                >
                  {shortAddress(backendSignerAddress)}
                </a>
              ) : (
                <Badge variant="neutral">Locked</Badge>
              )}
            </Row>
            <Row label="PLASMA balance">
              {backendPlasmaBalance !== undefined
                ? `${backendPlasmaBalance} PLASMA`
                : "—"}
            </Row>
            {whitelist && whitelist.length > 0 && (
              <div className="py-2">
                <span className="text-sm text-muted">Whitelist (per record type)</span>
                <div className="mt-2 flex flex-wrap gap-1.5">
                  {whitelist.map((w) => (
                    <Badge key={w.recordType} variant={w.whitelisted ? "success" : "neutral"}>
                      {w.recordType}
                    </Badge>
                  ))}
                </div>
              </div>
            )}
          </>
        )}
      </CardContent>
    </Card>
  );
}

/** helper to format a raw wei balance string to PLASMA for backend-mode callers */
export function formatPlasma(wei: bigint): string {
  return Number(formatUnits(wei, 18)).toFixed(4);
}
