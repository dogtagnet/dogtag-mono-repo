import type {
  ActivityIssuersResp,
  ActivityQuery,
  ActivityResp,
  ActivityStats,
  AdminActivityIssuersResp,
  AdminLoginResp,
  ApiError,
  ApproveApplicationResp,
  BusinessesQuery,
  BusinessesResp,
  BusinessLocationReviewResp,
  CreateIssuerReq,
  CreateIssuerResp,
  DelistApplicationResp,
  DirectoryResp,
  DirectorySigner,
  GovernanceAuthority,
  GovernanceAuthorityResp,
  IndexerStatus,
  IssuerApplicationReq,
  IssuerApplicationResp,
  IssuerApplicationsResp,
  PredictIssuerReq,
  PredictIssuerResp,
  RegisterBusinessReq,
  RegisterBusinessResp,
  RejectApplicationResp,
  WhitelistActionReq,
  WhitelistGrantResp,
  WhitelistRevokeResp,
  ProvidersResp,
  ProviderRegistrarView,
  RegisterProviderReq,
  RegisterProviderResp,
  ProviderStandingReq,
  ProviderStandingResp,
  ServiceApprovalReq,
  ServiceApprovalResp,
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

  /**
   * Build the `/v1/businesses` query string.
   *
   * This CANNOT send the caller's position, and that is the point rather than an omission. The route
   * still accepts `near=<lat>,<lng>` and `radius=` and still filters server-side (see the
   * deprecation note on `BusinessesQuery` in `stacks/admin/api/src/routes.rs`), but it is public and
   * unauthenticated, so anything sent there arrives beside the caller's IP with no account attached.
   * In a product built on the owner never revealing where they are, an endpoint whose purpose is to
   * be told where the user is should not be one argument away from being used.
   *
   * The replacement is `packages/ui/src/geo/`: fetch the provider set - the request is identical
   * whoever makes it - and compute distance on the device. Passing `near` here is the obvious way to
   * build "nearby", it would work on the first try, and it would turn a latent leak into a live one
   * in a single line. Do not re-add it.
   */
  function qs(q?: BusinessesQuery): string {
    if (!q) return "";
    const p = new URLSearchParams();
    if (q.type) p.set("type", q.type);
    const s = p.toString();
    return s ? `?${s}` : "";
  }

  /** Build the `/v1/admin/activity` query string - only set, non-empty filters are emitted. */
  function activityQs(q?: ActivityQuery): string {
    if (!q) return "";
    const p = new URLSearchParams();
    if (q.type) p.set("type", q.type);
    if (q.signer?.trim()) p.set("signer", q.signer.trim());
    if (q.issuer?.trim()) p.set("issuer", q.issuer.trim());
    if (q.recordType?.trim()) p.set("recordType", q.recordType.trim());
    if (q.root?.trim()) p.set("root", q.root.trim());
    if (q.dogTagId?.trim()) p.set("dogTagId", q.dogTagId.trim());
    if (q.finality) p.set("finality", q.finality);
    if (q.since !== undefined) p.set("since", String(q.since));
    if (q.until !== undefined) p.set("until", String(q.until));
    if (q.limit !== undefined) p.set("limit", String(q.limit));
    if (q.offset !== undefined) p.set("offset", String(q.offset));
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
    /**
     * GET /v1/admin/businesses/location-review — rows stored at exactly `0, 0`.
     *
     * Read-only, and it decides nothing: `0, 0` is a legal coordinate AND the value every blank
     * location was stored as before location became optional, so code cannot tell the two apart.
     * Each listed row needs an operator's answer.
     */
    businessesLocationReview: () =>
      request<BusinessLocationReviewResp>("GET", "/v1/admin/businesses/location-review"),

    // ---- issuer applications queue (§4.3) ----
    /** POST /v1/issuer-applications — create an application (public; no auth required). */
    createApplication: (body: IssuerApplicationReq) =>
      request<IssuerApplicationResp>("POST", "/v1/issuer-applications", body, "none"),
    listApplications: () =>
      request<IssuerApplicationsResp>("GET", "/v1/issuer-applications"),
    /**
     * POST /v1/issuer-applications/{id}/approve.
     *
     * The DNS legitimacy check is ADVISORY. When the live observation is not `verified` and
     * `proceedWithoutDns` is not set, the backend answers 409 `dnsConfirmationRequired` with the
     * observed state — the caller shows it and re-issues with `proceedWithoutDns: true` if the admin
     * deliberately confirms. Proceeding is recorded on the application, so an override is never
     * indistinguishable from a clean pass.
     */
    approveApplication: (id: string, opts?: { proceedWithoutDns?: boolean }) =>
      request<ApproveApplicationResp>("POST", `/v1/issuer-applications/${id}/approve`, {
        proceedWithoutDns: opts?.proceedWithoutDns ?? false,
      }),
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

    // ---- oversight indexer (PR-B data layer; the "see on-chain activity" surface) ----
    /** GET /v1/admin/activity - the UNSCOPED cross-issuer event feed, named by the admin directory. */
    getActivity: (q?: ActivityQuery) =>
      request<ActivityResp>("GET", `/v1/admin/activity${activityQs(q)}`),
    /** GET /v1/admin/activity/stats - cross-issuer aggregate counters (dashboard). */
    getActivityStats: () =>
      request<ActivityStats>("GET", "/v1/admin/activity/stats"),
    /** GET /v1/admin/activity/status - indexer progress + finality watermark (chain-health card). */
    getActivityStatus: () =>
      request<IndexerStatus>("GET", "/v1/admin/activity/status"),
    /** GET /v1/admin/activity/issuers - per-clone issued/revoked/active counts. */
    getActivityIssuers: () =>
      request<ActivityIssuersResp>("GET", "/v1/admin/activity/issuers"),
    /** GET /v1/admin/directory - the full signer→business directory (naming source). */
    getDirectory: () => request<DirectoryResp>("GET", "/v1/admin/directory"),
    /** GET /v1/admin/directory/signer/:addr - resolve one signer/clone address to its business. */
    getDirectorySigner: (addr: string) =>
      request<DirectorySigner>("GET", `/v1/admin/directory/signer/${addr}`),
    /** GET /v1/admin/governance/authority - the live on-chain authority map (chain-health card). */
    getGovernanceAuthority: () =>
      request<GovernanceAuthority>("GET", "/v1/admin/governance/authority"),

    // ---- direct whitelist management (PR-E) ----
    /** POST /v1/admin/whitelist/grant — whitelist a (signer, capability) pair via GovernanceAction. */
    whitelistGrant: (body: WhitelistActionReq) =>
      request<WhitelistGrantResp>("POST", "/v1/admin/whitelist/grant", body),
    /** POST /v1/admin/whitelist/revoke — delist a (signer, capability) pair via GovernanceAction. */
    whitelistRevoke: (body: WhitelistActionReq) =>
      request<WhitelistRevokeResp>("POST", "/v1/admin/whitelist/revoke", body),

    // ---- the generation-2 ProviderRegistry registrar surface (registry plan C-2) ----
    /** GET /v1/admin/providers - every registered provider, its standing, anchor and approvals. */
    listProviders: () => request<ProvidersResp>("GET", "/v1/admin/providers"),
    /** GET /v1/admin/providers/:providerId - one provider's registrar view. */
    getProvider: (providerId: string) =>
      request<ProviderRegistrarView>("GET", `/v1/admin/providers/${providerId}`),
    /** POST /v1/admin/providers - register a provider (the KYC gate), via GovernanceAction. */
    registerProvider: (body: RegisterProviderReq) =>
      request<RegisterProviderResp>("POST", "/v1/admin/providers", body),
    /**
     * POST /v1/admin/providers/:providerId/standing - move a provider's standing.
     * Required after registration: a provider is PENDING until this makes it ACTIVE.
     */
    setProviderStanding: (providerId: string, body: ProviderStandingReq) =>
      request<ProviderStandingResp>("POST", `/v1/admin/providers/${providerId}/standing`, body),
    /** POST /v1/admin/providers/:providerId/service-approval - approve/withdraw one record type. */
    setServiceCreationApproval: (providerId: string, body: ServiceApprovalReq) =>
      request<ServiceApprovalResp>(
        "POST",
        `/v1/admin/providers/${providerId}/service-approval`,
        body,
      ),
  };
}

export type CentralClient = ReturnType<typeof createCentralClient>;
