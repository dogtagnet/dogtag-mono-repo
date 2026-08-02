import { CheckCircle2, Lock, ShieldCheck, Sparkles, XCircle } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import type { ApiClient } from "../api/client";
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
import { explorerTxUrl } from "../wallet/chain";

export interface VerifyPurpose {
  value: string;
  label: string;
  recordType: string;
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
   * When true, shows a "Fill demo data" demo button that selects a purpose so a non-technical operator
   * can one-click prepare the verify session.
   *
   * Defaults FALSE, and the default is the safety property rather than a style choice: this component
   * is shared by production portals, so a demo-fill affordance inheriting `true` renders live and
   * invites fabricated data into a real system. A caller that wants it must say so - pass its own
   * `env.demoMode` - and a caller added later inherits the closed state rather than the open one.
   */
  showDemo?: boolean;
  /**
   * OPTIONAL shop appointment this verification is being performed for. Passed straight through to
   * `POST /verify/session/start`, which links the resulting verification to that appointment and its
   * client in the shop's history. Omit for an ad-hoc verification.
   */
  appointmentId?: string;
  /** Called once the verification reaches a terminal state, so the host can refresh its own view. */
  onSettled?: (result: { sessionId: string; status: "recorded" | "error"; txHash?: string }) => void;
}

type Phase = "idle" | "starting" | "awaiting" | "verified" | "error" | "failed";

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
  showDemo = false,
  appointmentId,
  onSettled,
}: VerifyFlowProps) {
  const [purpose, setPurpose] = useState<string>(purposes[0]?.value ?? "");
  const selected = purposes.find((p) => p.value === purpose);
  const [phase, setPhase] = useState<Phase>("idle");
  const [qrUrl, setQrUrl] = useState<string | null>(null);
  const [sessionId, setSessionId] = useState<string | null>(null);
  const [txHash, setTxHash] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const timer = useRef<ReturnType<typeof setInterval> | null>(null);

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
        appointmentId,
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
        // True success is ONLY status === "recorded". On error the backend stores the
        // failure message in the txHash field, so a non-empty txHash must NOT be treated
        // as success — surface it as a FAILED panel instead.
        if (s.status === "error") {
          setError(s.txHash || "Verification failed.");
          setTxHash(null);
          setPhase("failed");
          stopPolling();
          onSettled?.({ sessionId: id, status: "error" });
        } else if (s.status === "recorded") {
          setTxHash(s.txHash ?? null);
          setPhase("verified");
          stopPolling();
          onSettled?.({ sessionId: id, status: "recorded", txHash: s.txHash });
        }
      } catch {
        /* keep polling; transient errors are non-fatal */
      }
    }, pollIntervalMs);
  }

  function fillSample() {
    const preset = purposes[0];
    if (!preset) return;
    setPurpose(preset.value);
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

  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex items-center gap-2">
          <ShieldCheck className="h-5 w-5 text-primary" />
          Export on chain
        </CardTitle>
        <CardDescription>
          Start a session, let the owner scan and export a proof, then record the verification on ROAX.
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

            <div className="flex items-start gap-2 rounded-md border border-border bg-surface-muted p-3 text-sm">
              <Lock className="mt-0.5 h-4 w-4 shrink-0 text-primary" />
              <span className="text-muted">
                The owner approves an owner-hidden proof. Neither their identity nor the credential's
                contents are revealed, and no credential data is written on chain.
              </span>
            </div>

            {error && <p className="text-sm text-danger">{error}</p>}

            <div className="flex flex-wrap items-center gap-2">
              <Button onClick={() => void start()} loading={phase === "starting"} disabled={!selected}>
                Start export
              </Button>
              {showDemo && (
                <Button type="button" variant="outline" onClick={fillSample}>
                  <Sparkles className="h-4 w-4" /> Fill demo data
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
              The owner scans this QR and approves the private proof in their app — no credential
              data will be written on chain.
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
            <p className="text-sm text-muted">Private — no credential data on chain.</p>
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
              New export
            </Button>
          </div>
        )}

        {phase === "failed" && (
          <div className="flex flex-col items-center gap-3 py-4 text-center">
            <XCircle className="h-10 w-10 text-danger" />
            <p className="text-lg font-semibold">Verification failed</p>
            <p className="break-words text-sm text-danger">
              {error ?? "The verification could not be recorded on chain."}
            </p>
            <Button variant="outline" size="sm" onClick={reset}>
              Try again
            </Button>
          </div>
        )}
      </CardContent>
    </Card>
  );
}
