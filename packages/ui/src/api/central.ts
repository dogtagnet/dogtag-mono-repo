import type {
  AdminActivityIssuersResp,
  AdminLoginResp,
  ApiError,
  ApproveApplicationResp,
  BusinessesQuery,
  BusinessesResp,
  CreateIssuerReq,
  CreateIssuerResp,
  DelistApplicationResp,
  GovernanceAuthorityResp,
  IssuerApplicationReq,
  IssuerApplicationResp,
  IssuerApplicationsResp,
  PredictIssuerReq,
  PredictIssuerResp,
  RegisterBusinessReq,
  RegisterBusinessResp,
  RejectApplicationResp,
} from "./types";

export interface CentralClientOptions {
  /** central admin API base (e.g. "/api" with a Vite proxy, or an absolute origin) */
  baseUrl: string;
  /** returns the admin session bearer token, if logged in */
  getAdminToken?: () => string | null | undefined;
  /**
   * Invoked when an admin-gated request gets a 401 (stale session after a backend restart). The
   * host should clear the persisted admin token and route back to the admin login.
   */
  onUnauthorized?: () => void;
}

function makeError(status: number, body: unknown): ApiError {
  const msg =
    body && typeof body === "object" && "error" in body
      ? String((body as { error: unknown }).error)
      : `HTTP ${status}`;
  const e = new Error(msg) as ApiError;
  e.status = status;
  e.body = body;
  return e;
}

function safeJson(text: string): unknown {
  try {
    return JSON.parse(text);
  } catch {
    return text;
  }
}

/**
 * Typed client for the CENTRAL admin backend (`stacks/admin/api/src/routes.rs`). The admin portal
 * uses this for auth, business registry, the issuer-application queue, appointments and consents.
 * Distinct from `createApiClient` (the per-business vet/groomer backend client).
 */
export function createCentralClient(opts: CentralClientOptions) {
  const base = opts.baseUrl.replace(/\/$/, "");

  async function request<T>(
    method: string,
    path: string,
    body?: unknown,
    auth: "admin" | "none" = "admin",
  ): Promise<T> {
    const headers: Record<string, string> = { "content-type": "application/json" };
    if (auth === "admin") {
      const token = opts.getAdminToken?.();
      if (token) headers.authorization = `Bearer ${token}`;
    }
    const res = await fetch(`${base}${path}`, {
      method,
      headers,
      body: body === undefined ? undefined : JSON.stringify(body),
    });
    const text = await res.text();
    const parsed: unknown = text ? safeJson(text) : null;
    if (!res.ok) {
      // Stale admin session (backend restarted its in-memory session store) → clear + re-login.
      if (res.status === 401 && auth === "admin") opts.onUnauthorized?.();
      throw makeError(res.status, parsed);
    }
    return parsed as T;
  }

  function qs(q?: BusinessesQuery): string {
    if (!q) return "";
    const p = new URLSearchParams();
    if (q.type) p.set("type", q.type);
    if (q.near) p.set("near", q.near);
    if (q.radius !== undefined) p.set("radius", String(q.radius));
    const s = p.toString();
    return s ? `?${s}` : "";
  }

  return {
    base,

    // ---- admin auth ----
    adminLogin: (password: string) =>
      request<AdminLoginResp>("POST", "/v1/admin/login", { password }, "none"),

    // ---- business registry (§4.2) ----
    /** GET /v1/businesses — public discovery (no auth required). */
    listBusinesses: (q?: BusinessesQuery) =>
      request<BusinessesResp>("GET", `/v1/businesses${qs(q)}`, undefined, "none"),
    /** POST /v1/businesses — admin-gated; returns the HMAC secret ONCE. */
    registerBusiness: (body: RegisterBusinessReq) =>
      request<RegisterBusinessResp>("POST", "/v1/businesses", body),

    // ---- issuer applications queue (§4.3) ----
    /** POST /v1/issuer-applications — create an application (public; no auth required). */
    createApplication: (body: IssuerApplicationReq) =>
      request<IssuerApplicationResp>("POST", "/v1/issuer-applications", body, "none"),
    listApplications: () =>
      request<IssuerApplicationsResp>("GET", "/v1/issuer-applications"),
    approveApplication: (id: string) =>
      request<ApproveApplicationResp>("POST", `/v1/issuer-applications/${id}/approve`),
    rejectApplication: (id: string) =>
      request<RejectApplicationResp>("POST", `/v1/issuer-applications/${id}/reject`),
    delistApplication: (id: string) =>
      request<DelistApplicationResp>("POST", `/v1/issuer-applications/${id}/delist`),

    // ---- control plane: factory deploys + governance authority (PR-A / PR-C) ----
    /** POST /v1/admin/factory/predict — deterministic clone-address preview (read-only, no tx). */
    predictIssuer: (body: PredictIssuerReq) =>
      request<PredictIssuerResp>("POST", "/v1/admin/factory/predict", body),
    /**
     * POST /v1/admin/factory/issuers — deploy an issuer clone through the GovernanceAction layer.
     * Returns `result.disposition === "executed"` (hosted key IS the factory owner → tx broadcast) or
     * `"proposed"` (ownership moved to governance → calldata payload for out-of-band execution).
     */
    createIssuer: (body: CreateIssuerReq) =>
      request<CreateIssuerResp>("POST", "/v1/admin/factory/issuers", body),
    /** GET /v1/admin/governance/authority — live authority map (factory owner / WHITELIST_ADMIN / DEFAULT_ADMIN). */
    governanceAuthority: () =>
      request<GovernanceAuthorityResp>("GET", "/v1/admin/governance/authority"),
    /** GET /v1/admin/activity/issuers — per-clone issued/revoked/active counts (needs the oversight indexer). */
    listIssuers: () =>
      request<AdminActivityIssuersResp>("GET", "/v1/admin/activity/issuers"),
  };
}

export type CentralClient = ReturnType<typeof createCentralClient>;
