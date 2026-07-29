import {
  Badge,
  Button,
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
  CredentialVerifyPanel,
  Label,
  QrCode,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
  Spinner,
  explorerTxUrl,
  useRoaxRpcPreference,
} from "@dogtag/ui";
import { CheckCircle2, History, RefreshCw, ShieldCheck, XCircle } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import { useApp } from "../app/AppContext";
import { env } from "../lib/env";

const PURPOSES = [
  {
    value: "boarding_intake",
    label: "Boarding intake — rabies status",
    recordType: "VACCINATION",
  },
  {
    value: "travel_check",
    label: "Travel check — vaccination",
    recordType: "VACCINATION",
  },
] as const;

type Phase = "idle" | "starting" | "awaiting" | "verified" | "error" | "failed";

interface HistoryItem {
  sessionId: string;
  purpose: string;
  recordType: string;
  status: string;
  txHash?: string | null;
  explorerUrl?: string | null;
  nullifier?: string | null;
  createdAt: number;
  updatedAt: number;
}

/** The sole owner-hidden verification flow. There is intentionally no disclosure choice. */
function OwnerHiddenVerifyFlow() {
  const { api } = useApp();
  const [purpose, setPurpose] = useState<string>(PURPOSES[0]?.value ?? "");
  const [phase, setPhase] = useState<Phase>("idle");
  const [qrUrl, setQrUrl] = useState<string | null>(null);
  const [txHash, setTxHash] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const timer = useRef<ReturnType<typeof setInterval> | null>(null);
  const selected = PURPOSES.find((item) => item.value === purpose);

  const stopPolling = useCallback(() => {
    if (timer.current) clearInterval(timer.current);
    timer.current = null;
  }, []);

  useEffect(() => () => stopPolling(), [stopPolling]);

  function reset() {
    stopPolling();
    setPhase("idle");
    setQrUrl(null);
    setTxHash(null);
    setError(null);
  }

  function beginPolling(sessionId: string) {
    stopPolling();
    timer.current = setInterval(async () => {
      try {
        const session = await api.verifySessionStatus(sessionId);
        if (session.status === "error") {
          setError(session.txHash || "Verification failed.");
          setPhase("failed");
          stopPolling();
        } else if (session.status === "recorded") {
          setTxHash(session.txHash ?? null);
          setPhase("verified");
          stopPolling();
        }
      } catch {
        // A transient poll failure is not a failed proof; keep polling the durable session.
      }
    }, 3000);
  }

  async function start() {
    if (!selected) return;
    setPhase("starting");
    setError(null);
    setTxHash(null);
    try {
      const session = await api.verifySessionStart({
        purpose: selected.value,
        recordType: selected.recordType,
      });
      setQrUrl(session.qrUrl);
      setPhase("awaiting");
      beginPolling(session.sessionId);
    } catch (cause) {
      setError((cause as Error).message);
      setPhase("error");
    }
  }

  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex items-center gap-2">
          <ShieldCheck className="h-5 w-5 text-primary" /> Request owner consent
        </CardTitle>
        <CardDescription>
          Start a session and let the owner prove consent without revealing their identity or
          credential contents.
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
                  {PURPOSES.map((item) => (
                    <SelectItem key={item.value} value={item.value}>
                      {item.label}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>
            <div className="rounded-md border border-border bg-surface-muted p-3 text-sm text-muted">
              Every request uses an owner-hidden zero-knowledge proof. The owner and credential
              contents stay private.
            </div>
            {error && <p className="text-sm text-danger">{error}</p>}
            <Button onClick={() => void start()} loading={phase === "starting"} disabled={!selected}>
              Start verification
            </Button>
          </>
        ) : null}

        {phase === "awaiting" && qrUrl && (
          <div className="flex flex-col items-center gap-4">
            <Badge variant="warning">
              <Spinner className="h-3 w-3" /> Awaiting owner consent
            </Badge>
            <QrCode value={qrUrl} caption={qrUrl} />
            <p className="text-center text-sm text-muted">
              Ask the owner to scan this request and approve the private proof on their device.
            </p>
            <Button variant="ghost" size="sm" onClick={reset}>
              Cancel
            </Button>
          </div>
        )}

        {phase === "verified" && (
          <div className="flex flex-col items-center gap-3 py-4 text-center">
            <CheckCircle2 className="h-10 w-10 text-success" />
            <p className="text-lg font-semibold">Verified</p>
            <p className="text-sm text-muted">The owner-hidden consent proof was recorded on ROAX.</p>
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

        {phase === "failed" && (
          <div className="flex flex-col items-center gap-3 py-4 text-center">
            <XCircle className="h-10 w-10 text-danger" />
            <p className="text-lg font-semibold">Verification failed</p>
            <p className="break-words text-sm text-danger">
              {error ?? "The proof could not be recorded."}
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

function statusVariant(status: string): "success" | "warning" | "danger" | "neutral" {
  if (status === "recorded") return "success";
  if (status === "error") return "danger";
  if (status === "recording" || status === "pending") return "warning";
  return "neutral";
}

function OwnerHiddenVerificationHistory() {
  const { api } = useApp();
  const [items, setItems] = useState<HistoryItem[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const response = await api.verificationHistory();
      setItems(response.verifications);
    } catch (cause) {
      setError((cause as Error).message);
    } finally {
      setLoading(false);
    }
  }, [api]);

  useEffect(() => {
    void load();
  }, [load]);

  return (
    <Card>
      <CardHeader>
        <div className="flex flex-col gap-3 sm:flex-row sm:items-start sm:justify-between">
          <div>
            <CardTitle className="flex items-center gap-2">
              <History className="h-5 w-5 text-primary" /> Verification history
            </CardTitle>
            <CardDescription>
              Recent owner-hidden consent verifications recorded by this vet portal.
            </CardDescription>
          </div>
          <Button
            type="button"
            variant="outline"
            size="sm"
            onClick={() => void load()}
            loading={loading}
          >
            <RefreshCw className="h-4 w-4" /> Refresh
          </Button>
        </div>
      </CardHeader>
      <CardContent className="space-y-3">
        {error && <p className="text-sm text-danger">{error}</p>}
        {items.length === 0 && !loading ? (
          <p className="rounded-md border border-dashed border-border p-4 text-sm text-muted">
            No verifications recorded yet.
          </p>
        ) : (
          items.map((item) => (
            <div
              key={item.sessionId}
              className="grid min-w-0 gap-2 rounded-md border border-border p-3 sm:grid-cols-[minmax(0,1fr)_auto]"
            >
              <div className="min-w-0">
                <p className="font-medium">{item.purpose}</p>
                <p className="text-xs text-muted">
                  {item.recordType} ·{" "}
                  {new Date((item.updatedAt || item.createdAt) * 1000).toLocaleString()}
                </p>
                {item.nullifier && (
                  <p className="mt-1 truncate font-mono text-xs text-muted" title={item.nullifier}>
                    nullifier {item.nullifier}
                  </p>
                )}
              </div>
              <div className="flex flex-wrap items-center gap-2 sm:justify-end">
                <Badge variant={statusVariant(item.status)}>{item.status}</Badge>
                {item.txHash && (
                  <a
                    href={item.explorerUrl || explorerTxUrl(item.txHash)}
                    target="_blank"
                    rel="noreferrer"
                    className="max-w-48 truncate font-mono text-xs text-primary hover:underline"
                    title={item.txHash}
                  >
                    {item.txHash}
                  </a>
                )}
              </div>
            </div>
          ))
        )}
      </CardContent>
    </Card>
  );
}

export function Verify() {
  const rpc = useRoaxRpcPreference(env.roaxRpc);
  return (
    <div className="space-y-4">
      <CredentialVerifyPanel rpcUrl={rpc.rpcUrl} defaultRpcUrl={rpc.defaultRpcUrl} />
      <OwnerHiddenVerifyFlow />
      <OwnerHiddenVerificationHistory />
    </div>
  );
}
