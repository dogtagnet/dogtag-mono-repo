import { CheckCircle2 } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import { Badge } from "../components/Badge";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "../components/Card";
import { Spinner } from "../components/Spinner";
import { explorerTxUrl } from "../wallet/chain";
import { isRootValid } from "../wallet/contracts";

export interface IssuanceStatusPanelProps {
  /** the issued credential's Merkle root (anchored on-chain) */
  root: string;
  /** confirmed issuance tx hash, if known (backend mode returns it on prepare/confirm) */
  txHash?: string;
  /**
   * the per-recordType issuer contract address (prepare's unsignedTx.to). When provided alongside
   * an RPC, the panel polls DogTagIssuer.isValid(root) to independently confirm the anchor on-chain,
   * transitioning Anchoring… → Verified on-chain ✓.
   */
  issuerAddr?: string;
  rpcUrl?: string;
  pollIntervalMs?: number;
}

type Phase = "anchoring" | "verified";

/**
 * On-chain issuance status (the portal mirror of the owner-app "poll until verified on chain" UX).
 * Backend mode anchors the Merkle root during prepare, so a txHash arrives immediately; the panel
 * still polls DogTagIssuer.isValid(root) (when an issuer address + RPC are configured) to surface a
 * live confirmation, then shows the explorer link.
 */
export function IssuanceStatusPanel({
  root,
  txHash,
  issuerAddr,
  rpcUrl,
  pollIntervalMs = 4000,
}: IssuanceStatusPanelProps) {
  const canPoll = Boolean(issuerAddr);
  // If we already have a confirmed tx and cannot poll, treat as verified.
  const [phase, setPhase] = useState<Phase>(txHash && !canPoll ? "verified" : "anchoring");
  const [onChainConfirmed, setOnChainConfirmed] = useState(false);
  const timer = useRef<ReturnType<typeof setInterval> | null>(null);

  const stop = useCallback(() => {
    if (timer.current) {
      clearInterval(timer.current);
      timer.current = null;
    }
  }, []);

  useEffect(() => {
    if (!canPoll || !issuerAddr) return;
    let cancelled = false;
    const tick = async () => {
      try {
        const valid = await isRootValid({ issuerAddr, root, rpcUrl });
        if (!cancelled && valid) {
          setOnChainConfirmed(true);
          setPhase("verified");
          stop();
        }
      } catch {
        /* transient RPC error — keep polling */
      }
    };
    void tick();
    timer.current = setInterval(() => void tick(), pollIntervalMs);
    return () => {
      cancelled = true;
      stop();
    };
  }, [canPoll, issuerAddr, root, rpcUrl, pollIntervalMs, stop]);

  const verified = phase === "verified";

  return (
    <Card>
      <CardHeader>
        <CardTitle className="text-base">On-chain status</CardTitle>
        <CardDescription>
          The Merkle root is anchored on ROAX via <code>DogTagIssuer</code>. This mirrors the owner-app
          "poll until verified on chain" experience.
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-3">
        <div className="flex items-center gap-2">
          {verified ? (
            <Badge variant="success">
              <CheckCircle2 className="h-3.5 w-3.5" /> Verified on-chain
            </Badge>
          ) : (
            <Badge variant="warning">
              <Spinner className="h-3.5 w-3.5" /> Anchoring…
            </Badge>
          )}
          {verified && canPoll && (
            <span className="text-xs text-muted">
              {onChainConfirmed ? "DogTagIssuer.isValid(root) = true" : "confirmed"}
            </span>
          )}
        </div>

        <div className="space-y-1 text-sm">
          <div className="break-all">
            <span className="text-muted">Merkle root: </span>
            <span className="font-mono">{root}</span>
          </div>
          {txHash && (
            <div className="break-all">
              <span className="text-muted">Tx: </span>
              <a
                href={explorerTxUrl(txHash)}
                target="_blank"
                rel="noreferrer"
                className="font-mono text-primary hover:underline"
              >
                {txHash}
              </a>
            </div>
          )}
        </div>

        {!canPoll && !txHash && (
          <p className="text-xs text-muted">
            Configure the issuer address + ROAX RPC to poll <code>isValid(root)</code> live.
          </p>
        )}
      </CardContent>
    </Card>
  );
}
