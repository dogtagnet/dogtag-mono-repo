import { CheckCircle2, Lock, ShieldCheck, Sparkles } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import type { ApiClient } from "../api/client";
import type { VerifyMode } from "../api/types";
import { Badge } from "../components/Badge";
import { Button } from "../components/Button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "../components/Card";
import { Label } from "../components/Label";
import { QrCode } from "../components/QrCode";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "../components/Select";
import { Spinner } from "../components/Spinner";
import { cn } from "../lib/cn";
import { explorerTxUrl } from "../wallet/chain";

export interface VerifyPurpose {
  value: string;
  label: string;
  recordType: string;
  /** sensitive purposes default to ZK (no data on chain) */
  sensitive?: boolean;
}

export interface VerifyFlowProps {
  client: ApiClient;
  purposes: VerifyPurpose[];
  /**
   * Optional poller: given a sessionId, returns the latest on-chain status. The backend exposes
   * no GET session endpoint in routes.rs, so the host can supply one (or omit to skip polling and
   * rely on a manual refresh / future endpoint). Returning `recorded` flips the UI to Verified.
   */
  pollSession?: (sessionId: string) => Promise<{ status: string; txHash?: string }>;
  pollIntervalMs?: number;
  /**
   * When true, shows a "Fill sample" demo button that selects a sensible non-sensitive purpose +
   * Normal mode so a non-technical operator can one-click prepare the verify session. Defaults true.
   */
  showDemo?: boolean;
}

type Phase = "idle" | "starting" | "awaiting" | "verified" | "error";

/**
 * On-chain proof-of-verification flow (impl §3.9 / §5.1):
 * pick purpose + Normal/ZK toggle → POST /verify/session/start → render QR → poll session →
 * show on-chain status (pending → Verified + explorer link). ZK shows the privacy note.
 */
export function VerifyFlow({
  client,
  purposes,
  pollSession,
  pollIntervalMs = 3000,
  showDemo = true,
}: VerifyFlowProps) {
  const [purpose, setPurpose] = useState<string>(purposes[0]?.value ?? "");
  const selected = purposes.find((p) => p.value === purpose);
  const [mode, setMode] = useState<VerifyMode>(selected?.sensitive === false ? "normal" : "zk");
  const [phase, setPhase] = useState<Phase>("idle");
  const [qrUrl, setQrUrl] = useState<string | null>(null);
  const [sessionId, setSessionId] = useState<string | null>(null);
  const [txHash, setTxHash] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const timer = useRef<ReturnType<typeof setInterval> | null>(null);

  // keep ZK as default whenever the chosen purpose is sensitive
  useEffect(() => {
    if (selected?.sensitive === false) setMode((m) => m);
    else setMode("zk");
  }, [purpose]); // eslint-disable-line react-hooks/exhaustive-deps

  const stopPolling = useCallback(() => {
    if (timer.current) {
      clearInterval(timer.current);
      timer.current = null;
    }
  }, []);

  useEffect(() => () => stopPolling(), [stopPolling]);

  async function start() {
    if (!selected) return;
    setPhase("starting");
    setError(null);
    setTxHash(null);
    try {
      const resp = await client.verifySessionStart({
        purpose: selected.value,
        recordType: selected.recordType,
        mode,
      });
      setQrUrl(resp.qrUrl);
      setSessionId(resp.sessionId);
      setPhase("awaiting");
      if (pollSession) beginPolling(resp.sessionId);
    } catch (e) {
      setError((e as Error).message);
      setPhase("error");
    }
  }

  function beginPolling(id: string) {
    stopPolling();
    timer.current = setInterval(async () => {
      try {
        const s = await pollSession!(id);
        if (s.status === "recorded" || s.txHash) {
          setTxHash(s.txHash ?? null);
          setPhase("verified");
          stopPolling();
        }
      } catch {
        /* keep polling; transient errors are non-fatal */
      }
    }, pollIntervalMs);
  }

  function fillSample() {
    const preset = purposes.find((p) => p.sensitive === false) ?? purposes[0];
    if (!preset) return;
    setPurpose(preset.value);
    setMode(preset.sensitive === false ? "normal" : "zk");
    setError(null);
  }

  function reset() {
    stopPolling();
    setPhase("idle");
    setQrUrl(null);
    setSessionId(null);
    setTxHash(null);
    setError(null);
  }

  const isZk = mode === "zk";

  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex items-center gap-2">
          <ShieldCheck className="h-5 w-5 text-primary" />
          Verify on chain
        </CardTitle>
        <CardDescription>
          Start a session, let the owner scan and approve consent, then record the verification on ROAX.
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-5">
        {phase === "idle" || phase === "starting" || phase === "error" ? (
          <>
            <div className="space-y-2">
              <Label htmlFor="verify-purpose">Purpose</Label>
              <Select value={purpose} onValueChange={setPurpose}>
                <SelectTrigger id="verify-purpose">
                  <SelectValue placeholder="Choose a purpose" />
                </SelectTrigger>
                <SelectContent>
                  {purposes.map((p) => (
                    <SelectItem key={p.value} value={p.value}>
                      {p.label}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>

            <div className="space-y-2">
              <Label>Mode</Label>
              <div className="grid grid-cols-2 gap-2">
                <button
                  type="button"
                  onClick={() => setMode("zk")}
                  className={cn(
                    "flex flex-col items-start gap-1 rounded-md border p-3 text-left text-sm transition-colors",
                    isZk ? "border-primary bg-primary/5 ring-1 ring-primary" : "border-border hover:bg-surface-muted",
                  )}
                >
                  <span className="flex items-center gap-1.5 font-medium">
                    <Lock className="h-3.5 w-3.5" /> ZK (private)
                  </span>
                  <span className="text-xs text-muted">No credential data on chain. Default for sensitive purposes.</span>
                </button>
                <button
                  type="button"
                  onClick={() => setMode("normal")}
                  className={cn(
                    "flex flex-col items-start gap-1 rounded-md border p-3 text-left text-sm transition-colors",
                    !isZk ? "border-primary bg-primary/5 ring-1 ring-primary" : "border-border hover:bg-surface-muted",
                  )}
                >
                  <span className="font-medium">Normal</span>
                  <span className="text-xs text-muted">Discloses the credential; reuses 3-pillar verify.</span>
                </button>
              </div>
            </div>

            {error && <p className="text-sm text-danger">{error}</p>}

            <div className="flex flex-wrap items-center gap-2">
              <Button onClick={() => void start()} loading={phase === "starting"} disabled={!selected}>
                Start verification
              </Button>
              {showDemo && (
                <Button type="button" variant="outline" onClick={fillSample}>
                  <Sparkles className="h-4 w-4" /> Fill sample
                </Button>
              )}
            </div>
          </>
        ) : null}

        {phase === "awaiting" && qrUrl && (
          <div className="flex flex-col items-center gap-4">
            <Badge variant="warning">
              <Spinner className="h-3 w-3" /> Awaiting owner consent
            </Badge>
            <QrCode value={qrUrl} caption={qrUrl} />
            <p className="text-center text-sm text-muted">
              {isZk
                ? "Private — no credential data will be written on chain."
                : "The owner scans this QR and approves disclosure in their app."}
            </p>
            {!pollSession && (
              <p className="text-center text-xs text-muted">
                Session {sessionId?.slice(0, 8)}… created. Status updates when the owner consent is relayed.
              </p>
            )}
            <Button variant="ghost" size="sm" onClick={reset}>
              Cancel
            </Button>
          </div>
        )}

        {phase === "verified" && (
          <div className="flex flex-col items-center gap-3 py-4 text-center">
            <CheckCircle2 className="h-10 w-10 text-success" />
            <p className="text-lg font-semibold">Verified</p>
            {isZk && (
              <p className="text-sm text-muted">Private — no credential data on chain.</p>
            )}
            {txHash && (
              <a
                href={explorerTxUrl(txHash)}
                target="_blank"
                rel="noreferrer"
                className="break-all font-mono text-sm text-primary hover:underline"
              >
                {txHash}
              </a>
            )}
            <Button variant="outline" size="sm" onClick={reset}>
              New verification
            </Button>
          </div>
        )}
      </CardContent>
    </Card>
  );
}
