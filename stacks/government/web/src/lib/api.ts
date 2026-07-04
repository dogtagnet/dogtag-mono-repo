/** Government-portal API client + shared types.
 *
 * `API_BASE` is the authenticated JSON surface (`/api` → backend, proxied by vite/nginx). The public
 * PII-free receipt status page lives at same-origin `/r/:receiptId` (also proxied to the backend);
 * that is the URL the receipt QR encodes so a verifier can confirm VALID/EXPIRED/REVOKED with no PII.
 */
export const API_BASE = import.meta.env.VITE_GOV_API_BASE || "/api";

// Bearer token for the operator endpoints (issue, records, PATCH, revoke). The fallback matches the
// backend's demo-mode default so the demo portal works out of the box; set VITE_GOV_API_TOKEN
// (and GOV_API_TOKEN on the API) for a real deployment.
export const API_TOKEN = import.meta.env.VITE_GOV_API_TOKEN || "dogtag-gov-demo-token";

export async function apiGet(path: string) {
  const r = await fetch(`${API_BASE}${path}`, {
    headers: { authorization: `Bearer ${API_TOKEN}` },
  });
  if (!r.ok) {
    const body = await r.json().catch(() => null);
    throw new Error(body?.error || `HTTP ${r.status}`);
  }
  return r.json();
}

export async function apiPost(path: string, body: unknown, opts?: { auth?: boolean }) {
  const headers: Record<string, string> = { "content-type": "application/json" };
  if (opts?.auth) headers.authorization = `Bearer ${API_TOKEN}`;
  const r = await fetch(`${API_BASE}${path}`, {
    method: "POST",
    headers,
    body: JSON.stringify(body),
  });
  return { status: r.status, json: await r.json() };
}

export async function apiPatch(path: string, body: unknown) {
  const r = await fetch(`${API_BASE}${path}`, {
    method: "PATCH",
    headers: { "content-type": "application/json", authorization: `Bearer ${API_TOKEN}` },
    body: JSON.stringify(body),
  });
  return { status: r.status, json: await r.json() };
}

export interface Health {
  status?: string;
  chainId?: number;
  demo?: boolean;
  canSign?: boolean;
  signer?: string | null;
}

/** A persisted government credential as returned by GET /v1/records (serde camelCase). */
export interface GovRecord {
  root: string;
  recordType: string;
  dogTagId: string;
  issuerAddr: string;
  receiptId?: string | null;
  /** Cleartext credentialSubject projection (Section A/B/C + validity) — custodial, authenticated. */
  subject?: Record<string, unknown> | null;
  validUntil?: string | null;
  wrappedDoc?: unknown;
  status: string;
  /** Read-time derived VALID / EXPIRED / REVOKED / DRAFT verdict injected by the backend. */
  effectiveStatus?: string;
  txHash?: string | null;
  blockNumber?: number | null;
  explorerUrl?: string | null;
  anchored?: boolean;
  label?: string | null;
  notes?: string | null;
  revokedTxHash?: string | null;
  revokeExplorerUrl?: string | null;
  invalidationReason?: string | null;
  createdAt?: number;
  updatedAt?: number;
}

/** PII-free public receipt status (GET /v1/receipts/:receiptId/status) — a LIVE on-chain read. */
export interface ReceiptStatus {
  effectiveStatus: string;
  recordType: string;
  receiptId: string;
  validUntil?: string | null;
  issuanceDate?: string | null;
  root: string;
  issuerAddr: string;
  explorerUrl?: string | null;
  revokeExplorerUrl?: string | null;
  checkedAt: number;
}

/** The public, PII-free status URL for a receipt (what the QR encodes). Same origin as the portal;
 *  proxied to the backend's server-rendered `/r/:receiptId` page (vite dev + nginx prod). */
export function publicReceiptUrl(receiptId: string): string {
  const origin = typeof window !== "undefined" ? window.location.origin : "";
  return `${origin}/r/${receiptId}`;
}

/** Map a derived status verdict to a @dogtag/ui Badge variant. */
export function statusVariant(
  effective: string | undefined,
): "success" | "warning" | "danger" | "neutral" {
  switch ((effective || "").toUpperCase()) {
    case "VALID":
      return "success";
    case "EXPIRED":
      return "warning";
    case "REVOKED":
      return "danger";
    default:
      return "neutral";
  }
}

/** A fixed hex accent for the receipt-sheet status chip (the sheet uses a print-safe fixed palette,
 *  not the theme tokens). Mirrors the public `/r/:id` page colors. */
export function statusAccent(effective: string | undefined): string {
  switch ((effective || "").toUpperCase()) {
    case "VALID":
      return "#16a34a";
    case "EXPIRED":
      return "#d97706";
    case "REVOKED":
      return "#dc2626";
    default:
      return "#64748b";
  }
}
