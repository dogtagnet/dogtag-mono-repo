import { useEffect, useState } from "react";

/**
 * The moment a freshly minted one-time token lapses, from its `ttlSecs` at RESPONSE RECEIPT.
 *
 * Deliberately not `expiresAt`: that is the backend's clock and this is the browser's, and even a few
 * seconds of skew between them would show a wrong (or already expired) timer over a QR that is perfectly
 * good. `expiresAt` stays the underlying fact to display; `ttlSecs` is the countdown basis.
 */
export function deadlineFromTtl(ttlSecs: number): number {
  return Date.now() + ttlSecs * 1000;
}

/** Whole seconds left until `deadline` (epoch ms), or 0 when nothing is counting. */
export function useCountdown(deadline: number | null): number {
  const [, tick] = useState(0);
  useEffect(() => {
    if (deadline === null) return;
    const timer = setInterval(() => tick((n) => n + 1), 1000);
    return () => clearInterval(timer);
  }, [deadline]);
  return deadline === null ? 0 : Math.max(0, Math.ceil((deadline - Date.now()) / 1000));
}

/** `m:ss`, for a countdown badge. */
export function mmss(secs: number): string {
  return `${Math.floor(secs / 60)}:${String(secs % 60).padStart(2, "0")}`;
}
