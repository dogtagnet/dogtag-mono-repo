import { AlertTriangle, CheckCircle2, HelpCircle, XCircle } from "lucide-react";
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

/** How one whitelist row renders. Exported so the decision has one definition and one set of tests. */
export interface WhitelistBadge {
  tone: "success" | "neutral" | "warning";
  /** The record type, plus - for the third state ONLY - what could not be established about it. */
  label: string;
  icon: "granted" | "refused" | "unresolved";
}

/**
 * THE decision behind a whitelist badge, over the wire's THREE states.
 *
 * `null` is "we could not check", and it gets its own tone AND its own words. Not a tooltip and not a
 * `title` attribute: a state legible only on hover survives neither a screenshot, nor a touch device,
 * nor a paste into an incident report - the same reason the admin portal's inert explorer links carry
 * visible text rather than a hover. And not the red X either, which says the authority answered and
 * refused this signer; an operator shown that goes looking for a permissions problem that does not
 * exist, while the real fault is their RPC.
 *
 * `warning` rather than a fourth tone: this repo already spells "unresolved" amber - the verification
 * bench's could-not-run rows and `CredentialVerifyPanel`'s Unresolved treatment - and a new colour for
 * the same meaning would be a second vocabulary to keep in step.
 *
 * Pure and exhaustive over the three states, so both render sites below share ONE rule; backend mode
 * previously distinguished nothing at all, printing the same neutral chip whatever the answer.
 */
export function whitelistBadge(row: Pick<WhitelistRow, "recordType" | "whitelisted">): WhitelistBadge {
  if (row.whitelisted === null || row.whitelisted === undefined) {
    return {
      tone: "warning",
      label: `${row.recordType} — could not check`,
      icon: "unresolved",
    };
  }
  return row.whitelisted
    ? { tone: "success", label: row.recordType, icon: "granted" }
    : { tone: "neutral", label: `${row.recordType} — not approved`, icon: "refused" };
}

function WhitelistBadges({ rows }: { rows: WhitelistRow[] }) {
  return (
    <div className="mt-2 flex flex-wrap gap-1.5">
      {rows.map((w) => {
        const b = whitelistBadge(w);
        return (
          <Badge key={w.recordType} variant={b.tone}>
            {b.icon === "granted" ? (
              <CheckCircle2 className="h-3 w-3" />
            ) : b.icon === "refused" ? (
              <XCircle className="h-3 w-3" />
            ) : (
              <HelpCircle className="h-3 w-3" />
            )}
            {b.label}
          </Badge>
        );
      })}
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
              {whitelist && whitelist.length > 0 ? (
                <WhitelistBadges rows={whitelist} />
              ) : (
                <div className="mt-2 flex flex-wrap gap-1.5">
                  <span className="text-sm text-muted">No record types configured</span>
                </div>
              )}
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
                <WhitelistBadges rows={whitelist} />
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
