import {
  Badge,
  Button,
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
  QrCode,
} from "@dogtag/ui";
import { RefreshCw } from "lucide-react";
import { useEffect, useState } from "react";
import { shareRecord, type QrToken } from "../lib/api";

/** The live countdown for a one-time QR token.
 *
 *  Counted from `ttlSecs` measured at RESPONSE RECEIPT, not `expiresAt - Date.now()`: the two clocks
 *  are the backend's and the browser's, and even a few seconds of skew would show a wrong (or already
 *  expired) timer on a QR that is perfectly good. `expiresAt` is still shown as the underlying fact. */
function useCountdown(token: QrToken | null): number {
  const [remaining, setRemaining] = useState(0);
  useEffect(() => {
    if (!token) return;
    const deadline = Date.now() + token.ttlSecs * 1000;
    const tick = () => setRemaining(Math.max(0, Math.ceil((deadline - Date.now()) / 1000)));
    tick();
    const timer = setInterval(tick, 1000);
    return () => clearInterval(timer);
  }, [token]);
  return remaining;
}

function mmss(secs: number): string {
  const m = Math.floor(secs / 60);
  const s = secs % 60;
  return `${m}:${String(s).padStart(2, "0")}`;
}

export interface ShareQrTarget {
  root: string;
  /** What the operator sees named in the dialog (receipt id, or a truncated root). */
  label: string;
}

/**
 * The record-share QR: the owner's phone scans it and imports the credential.
 *
 * The token is ONE-TIME and short-lived — it is consumed by the first scan and expires on its own — so
 * this dialog makes both facts visible: a live countdown while it is good, an unmistakable expired
 * state after, and a button that mints a fresh token in place.
 *
 * The URL comes from the backend (built from `DEPLOYMENT_URL`), never from `window.location`: the
 * portal's own origin is frequently a laptop's `localhost`, which no phone can reach.
 */
export function ShareQrDialog({
  target,
  onClose,
}: {
  target: ShareQrTarget | null;
  onClose: () => void;
}) {
  const [token, setToken] = useState<QrToken | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const remaining = useCountdown(token);
  const expired = token !== null && remaining === 0;

  const root = target?.root ?? null;
  useEffect(() => {
    if (!root) {
      setToken(null);
      setError(null);
      return;
    }
    let cancelled = false;
    setBusy(true);
    setError(null);
    setToken(null);
    shareRecord(root)
      .then((t) => {
        if (!cancelled) setToken(t);
      })
      .catch((e) => {
        if (!cancelled) setError(String(e instanceof Error ? e.message : e));
      })
      .finally(() => {
        if (!cancelled) setBusy(false);
      });
    return () => {
      cancelled = true;
    };
  }, [root]);

  async function mintFresh() {
    if (!root) return;
    setBusy(true);
    setError(null);
    try {
      setToken(await shareRecord(root));
    } catch (e) {
      setError(String(e instanceof Error ? e.message : e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <Dialog open={target !== null} onOpenChange={(open) => !open && onClose()}>
      <DialogContent className="flex flex-col items-center" data-testid="share-qr-dialog">
        <DialogHeader>
          <DialogTitle>Share QR</DialogTitle>
          <DialogDescription>
            One-time — the first scan consumes it. The owner scans this in their DogTag app to import{" "}
            {target?.label ? <strong>{target.label}</strong> : "the credential"}.
          </DialogDescription>
        </DialogHeader>

        {error && (
          <Badge variant="danger" data-testid="share-qr-error">
            {error}
          </Badge>
        )}

        {busy && !token && <p className="py-6 text-sm text-muted">Minting a one-time token…</p>}

        {token && !expired && (
          <div className="flex flex-col items-center gap-3">
            <QrCode value={token.qrUrl} caption={token.qrUrl} />
            <Badge variant={remaining <= 30 ? "warning" : "neutral"} data-testid="share-qr-countdown">
              expires in {mmss(remaining)}
            </Badge>
          </div>
        )}

        {expired && (
          <div className="flex flex-col items-center gap-3 py-6 text-center" data-testid="share-qr-expired">
            <Badge variant="danger">QR expired</Badge>
            <p className="max-w-xs text-sm text-muted">
              This one-time token has lapsed and can no longer be scanned. Mint a fresh one to try
              again.
            </p>
          </div>
        )}

        <Button
          variant="outline"
          size="sm"
          className="mt-2"
          data-testid="share-qr-refresh"
          loading={busy}
          disabled={busy || !root}
          onClick={() => void mintFresh()}
        >
          <RefreshCw className="h-4 w-4" /> New QR
        </Button>
      </DialogContent>
    </Dialog>
  );
}
