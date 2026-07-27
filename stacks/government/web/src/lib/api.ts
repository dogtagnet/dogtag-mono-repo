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

/** A GET that surfaces the HTTP status so the caller can render a first-class 503 "oversight indexer
 *  not connected" state (rather than a generic thrown error). */
export async function apiGetResult(path: string): Promise<{ status: number; json: unknown }> {
  const r = await fetch(`${API_BASE}${path}`, {
    headers: { authorization: `Bearer ${API_TOKEN}` },
  });
  const json = await r.json().catch(() => null);
  return { status: r.status, json };
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
  /** "live" (real RPC node) or "simulated" (in-process MemChain — nothing is broadcast). */
  backend?: "live" | "simulated";
  /** Same fact as `backend`, for badge logic. */
  simulated?: boolean;
  /** The real EIP-155 id when live; `null` when simulated (the backend is on no network). */
  chainId?: number | null;
  /** Ephemeral store + relaxed API token. NOT a statement about the chain — see `backend`. */
  demo?: boolean;
  /** True only when a real tx from a real key would land on a real chain. */
  canSign?: boolean;
  /** The real signer address; `null` when simulated. */
  signer?: string | null;
  /** The MemChain stand-in address; only set when simulated. */
  simulatedSigner?: string | null;
  /**
   * Per-record-type `DogTagIssuer` clone (the credential's documentStore), or `null` when this
   * deployment has none configured for that type — issuance of it fails closed with 503. Absent
   * entirely on older backends, so consumers must fail OPEN when a key is missing.
   */
  issuers?: Record<string, string | null>;
  /** `IssuerRegistry` — where the `VERIFY:<purpose>` whitelist is read from. */
  issuerRegistry?: string;
  /** The unified owner-hidden verification registry this authority submits consent proofs to. */
  verificationRegistry?: string;
  /** The host the QR codes point at. Must be reachable from the OWNER'S PHONE, not just this box. */
  deploymentUrl?: string;
}

// ---- phone handoff: one-time QR tokens (mirrors the vet/groomer contract exactly) ----

/** A minted one-time QR token. `ttlSecs` is the countdown basis; `expiresAt` is the fact to show. */
export interface QrToken {
  qrUrl: string;
  ttlSecs: number;
  expiresAt: number;
}

/** `POST /v1/records/:root/share` — mint a one-time record-share QR the owner scans to import. */
export async function shareRecord(root: string): Promise<QrToken> {
  const { status, json } = await apiPost(`/v1/records/${root}/share`, {}, { auth: true });
  if (status !== 200) throw new Error(json?.error || `HTTP ${status}`);
  return json as QrToken;
}

export interface VerifySessionStart extends QrToken {
  sessionId: string;
  purpose: string;
  recordType: string;
  relayer: string;
}

/** `POST /verify/session/start` — begin an owner-hidden verify session and get its QR.
 *
 *  Throws with the backend's own message so the portal can render the honest refusal (notably
 *  "relayer not whitelisted for this purpose" when this authority's signer has not been granted the
 *  `VERIFY:<purpose>` namespace on-chain). */
export async function verifySessionStart(
  purpose: string,
  recordType: string,
): Promise<VerifySessionStart> {
  const { status, json } = await apiPost(
    "/verify/session/start",
    { purpose, recordType },
    { auth: true },
  );
  if (status !== 200) throw new Error(json?.error || `HTTP ${status}`);
  return json as VerifySessionStart;
}

export interface VerifySessionStatus {
  status: "pending" | "recording" | "recorded" | "error" | string;
  txHash?: string | null;
  nullifier?: string | null;
  disclosedKeyPaths?: string[];
}

/** `GET /verify/session/:id` — poll a verify session (operator-gated; the phone polls with `?token=`). */
export async function verifySessionStatus(sessionId: string): Promise<VerifySessionStatus> {
  return (await apiGet(`/verify/session/${sessionId}`)) as VerifySessionStatus;
}

/** One owner-hidden verification this authority recorded. Proof metadata only — never owner identity. */
export interface VerifyHistoryItem {
  sessionId: string;
  relayer: string;
  purpose: string;
  recordType: string;
  status: string;
  txHash?: string | null;
  explorerUrl?: string | null;
  nullifier?: string | null;
  disclosedKeyPaths?: string[];
  createdAt: number;
  updatedAt: number;
}

/** `GET /verify/history` — the owner-hidden verification audit log. */
export async function verifyHistory(): Promise<VerifyHistoryItem[]> {
  const json = await apiGet("/verify/history");
  return (json.verifications as VerifyHistoryItem[]) ?? [];
}

/** The `VERIFY:<purpose>` namespaces this portal offers. Mirrors `government_api::app::VERIFY_PURPOSES`;
 *  `travel_check` is the border/port-of-entry check, a label already in circulation across the portals
 *  so an owner's consent receipt renders it by name rather than as a bare field hash.
 *
 *  Both entries share one purpose and differ only in the record type the owner is asked to present. */
export const VERIFY_PURPOSES = [
  {
    value: "travel_check",
    recordType: "VACCINATION",
    label: "Travel check — rabies vaccination",
  },
  {
    value: "travel_check",
    recordType: "EU_HEALTH_CERT",
    label: "Travel check — EU health certificate",
  },
] as const;

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

// ---- oversight console (govarch PR-5): UNSCOPED cross-issuer activity joined to own credentials ----

/** The government's own credential/verification joined to an on-chain event (non-PII summary). */
export interface OversightLocalJoin {
  kind: "issuance" | "verification";
  root?: string;
  recordType?: string;
  dogTagId?: string;
  receiptId?: string | null;
  status?: string;
  label?: string | null;
  id?: string;
  verdict?: boolean;
  /**
   * "dogTagId" when an owner-blind `verified` event (no root) joined via the field-hashed tag id:
   * the event proves a TAG this authority credentialed was verified - not which credential.
   */
  joinedBy?: string;
}

/** One non-PII on-chain oversight event (from the indexer), joined to the government's own records. */
export interface OversightEvent {
  id: string;
  type: string;
  /**
   * The address that EMITTED the log. NOT always the issuer clone: `issuerCreated` comes from the
   * factory, `whitelisted`/`delisted` from the IssuerRegistry, `verified` from the verification
   * registry. This is the answer to "which smart contract".
   */
  contract?: string;
  actor?: string;
  clone?: string;
  /** `issuerCreated`: the human name the clone was registered under. */
  name?: string;
  recordType?: string;
  root?: string;
  dogTagId?: string;
  /** `verified` events (owner-blind): the hashed purpose key the consent was proven for. */
  purpose?: string;
  /** `verified` events: the unlinkable consent nullifier. */
  nullifier?: string;
  /** `verified` events: the consent window's proof-bound deadline (unix seconds). */
  deadline?: number;
  txHash?: string;
  blockHash?: string;
  blockNumber?: number;
  blockTimestamp?: number;
  /** The block timestamp as the chain reported it; equals `blockTimestamp` on a healthy index. */
  onchainTs?: number;
  /** Position of this log within its transaction — disambiguates two events in one tx. */
  logIndex?: number;
  finality?: string;
  actorName?: string;
  cloneName?: string;
  txUrl?: string;
  /** The government's own record joined to this event, or null when it belongs to another issuer. */
  local?: OversightLocalJoin | null;
}

export interface OversightActivityResp {
  events: OversightEvent[];
  total?: number;
  /** how many cross-issuer events are the government's OWN credentials. */
  matched?: number;
  scope?: { label?: string; unscoped?: boolean };
}

export interface OversightStatsResp {
  rootIssued?: number;
  rootRevoked?: number;
  activeCredentials?: number;
  verifications?: number;
  clones?: number;
  signers?: number;
  local?: { credentials: number; verifications: number };
  [k: string]: unknown;
}

/** Map an on-chain event kind to a display label + Badge variant. */
export function eventMeta(t: string): {
  label: string;
  variant: "success" | "warning" | "danger" | "neutral" | "default";
} {
  switch (t) {
    case "rootIssued":
      return { label: "Root issued", variant: "success" };
    case "rootRevoked":
      return { label: "Root revoked", variant: "danger" };
    case "verified":
      return { label: "Verified", variant: "default" };
    case "whitelisted":
      return { label: "Whitelisted", variant: "success" };
    case "delisted":
      return { label: "Delisted", variant: "warning" };
    case "issuerCreated":
      return { label: "Issuer created", variant: "default" };
    case "rootRegistered":
      return { label: "Root registered", variant: "neutral" };
    default:
      return { label: t, variant: "neutral" };
  }
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
