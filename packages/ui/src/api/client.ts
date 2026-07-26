import type {
  AccountsReq,
  AccountsResp,
  ApiError,
  ConfirmReq,
  ConfirmResp,
  GenesisConfirmReq,
  GenesisConfirmResp,
  GenesisStartResp,
  ImportPullReq,
  ImportPullResp,
  IssuerApplicationReq,
  IssuerApplicationResp,
  IssuerSignersResp,
  LoginResp,
  AdminLoginResp,
  PrepareReq,
  PrepareResp,
  ProfileIssueStartReq,
  ProfileIssueStartResp,
  ProfileIssueStatusResp,
  RecordsListResp,
  RevokeResp,
  ShareResp,
  TraceActivityResp,
  TraceQuery,
  TraceStatsResp,
  UpdateRecordReq,
  SigningMode,
  SigningModeResp,
  UnlockReq,
  UnlockResp,
  VerifyConsentSubmitReq,
  VerifyConsentSubmitResp,
  VerificationHistoryResp,
  VerifyCredentialReq,
  VerifyCredentialResp,
  VerifySessionStartReq,
  VerifySessionStartResp,
  VerifySessionStatusResp,
} from "./types";
import { isCustodyLockedError } from "../custody/lock";

export interface ApiClientOptions {
  /** vet backend base URL (e.g. "/api" with a Vite proxy, or an absolute origin) */
  baseUrl: string;
  /** central admin API base (for issuer-application apply); optional */
  centralBaseUrl?: string;
  /** returns the operator session bearer token, if logged in */
  getOperatorToken?: () => string | null | undefined;
  /** returns the admin (custody) session bearer token, if logged in */
  getAdminToken?: () => string | null | undefined;
  /**
   * Invoked when a request that carried a stored token gets a 401. The backends use an in-memory
   * session store, so a backend restart silently invalidates the persisted token; the host should
   * CLEAR the matching token and route the user back to the relevant login step. Only fired for the
   * "operator"/"admin" token kinds (not record-JWT bearer or unauthenticated calls).
   */
  onUnauthorized?: (kind: "operator" | "admin") => void;
  /**
   * Invoked when ANY call is refused because the backend's custody seal is locked (see
   * `isCustodyLockedError`). Hosts use this to flip their custody state and route the operator to
   * the dedicated /unlock page instead of surfacing a dead-end error toast. Purely a client-side
   * signal — the backend's locked state and its gates are untouched.
   */
  onCustodyLocked?: () => void;
}

type TokenKind = "operator" | "admin" | "bearer" | "none";

/**
 * Paths whose 401 is a CREDENTIAL rejection, not a dead session, so `onUnauthorized` must not fire.
 * `/admin/unlock` answers a wrong passphrase with 401; clearing the just-issued admin token there
 * would replace the unlock page's inline "wrong passphrase" with a bogus "session expired".
 */
const CREDENTIAL_401_PATHS = new Set(["/admin/unlock"]);

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

export function createApiClient(opts: ApiClientOptions) {
  const base = opts.baseUrl.replace(/\/$/, "");
  const central = opts.centralBaseUrl?.replace(/\/$/, "");

  async function request<T>(
    method: string,
    path: string,
    body?: unknown,
    tokenKind: TokenKind = "operator",
    explicitToken?: string,
    rootBase: "vet" | "central" = "vet",
  ): Promise<T> {
    const root = rootBase === "central" ? central : base;
    if (!root) throw new Error(`No base URL configured for ${rootBase} API`);
    const headers: Record<string, string> = { "content-type": "application/json" };

    let token: string | null | undefined;
    if (explicitToken) token = explicitToken;
    else if (tokenKind === "operator") token = opts.getOperatorToken?.();
    else if (tokenKind === "admin") token = opts.getAdminToken?.();
    if (token && tokenKind !== "none") headers.authorization = `Bearer ${token}`;

    const res = await fetch(`${root}${path}`, {
      method,
      headers,
      body: body === undefined ? undefined : JSON.stringify(body),
    });

    const text = await res.text();
    const parsed: unknown = text ? safeJson(text) : null;
    if (!res.ok) {
      // Stale-session handling: a 401 on a token-bearing call means the persisted session was
      // invalidated (e.g. the backend restarted its in-memory session store). Clear it so the UI
      // routes back to login instead of replaying a dead token.
      if (
        res.status === 401 &&
        (tokenKind === "operator" || tokenKind === "admin") &&
        !explicitToken &&
        !CREDENTIAL_401_PATHS.has(path)
      ) {
        opts.onUnauthorized?.(tokenKind);
      }
      const e = makeError(res.status, parsed);
      // Locked-custody handling: a restart drops the decrypted seed, so any custody-backed call
      // starts failing with "not unlocked". Surface it once, centrally, so the host can redirect to
      // /unlock rather than every call site rendering its own dead-end toast.
      if (isCustodyLockedError(e)) opts.onCustodyLocked?.();
      throw e;
    }
    return parsed as T;
  }

  function safeJson(text: string): unknown {
    try {
      return JSON.parse(text);
    } catch {
      return text;
    }
  }

  /** Build a `?a=b&…` query string from a TraceQuery, skipping unset/blank fields. */
  function traceQs(q?: TraceQuery): string {
    if (!q) return "";
    const p = new URLSearchParams();
    for (const [k, v] of Object.entries(q)) {
      if (v !== undefined && v !== null && `${v}` !== "") p.set(k, `${v}`);
    }
    const s = p.toString();
    return s ? `?${s}` : "";
  }

  return {
    base,
    central,

    // ---- login ----
    login: (password: string) => request<LoginResp>("POST", "/login", { password }, "none"),
    adminLogin: (password: string) =>
      request<AdminLoginResp>("POST", "/admin/login", { password }, "none"),

    // ---- genesis / custody (admin session) ----
    genesisStart: () => request<GenesisStartResp>("POST", "/admin/genesis/start", undefined, "admin"),
    genesisConfirm: (body: GenesisConfirmReq) =>
      request<GenesisConfirmResp>("POST", "/admin/genesis/confirm", body, "admin"),
    unlock: (body: UnlockReq) => request<UnlockResp>("POST", "/admin/unlock", body, "admin"),
    addAccount: (body: AccountsReq) => request<AccountsResp>("POST", "/admin/accounts", body, "admin"),

    // ---- settings (operator session) ----
    getSigningMode: () => request<SigningModeResp>("GET", "/settings/signing-mode"),
    putSigningMode: (mode: SigningMode) =>
      request<SigningModeResp>("PUT", "/settings/signing-mode", { mode }),

    // ---- credentials ----
    prepare: (body: PrepareReq) => request<PrepareResp>("POST", "/credentials/prepare", body),
    confirm: (body: ConfirmReq) => request<ConfirmResp>("POST", "/credentials/confirm", body),

    // ---- traceability portal (govarch PR-5): this operator's on-chain activity joined to its DB ----
    /** GET /trace/activity — this operator's own on-chain credential activity (scoped server-side by
     *  the indexer + a local scope gate), joined to its own DB records. Operator-gated. */
    traceActivity: (q?: TraceQuery) =>
      request<TraceActivityResp>("GET", `/trace/activity${traceQs(q)}`),
    /** GET /trace/stats — this operator's in-scope on-chain counters + own record counts. */
    traceStats: () => request<TraceStatsResp>("GET", "/trace/stats"),

    // ---- records ----
    /** GET /records — list every record from the backend's OWN DB (operator-gated, most-recent first). */
    listRecords: () => request<RecordsListResp>("GET", "/records"),
    /** PATCH /records/:id — update OFF-CHAIN metadata only; the backend rejects on-chain-derived fields. */
    updateRecord: (id: string, body: UpdateRecordReq) =>
      request<Record<string, unknown>>("PATCH", `/records/${id}`, body),
    revoke: (id: string) => request<RevokeResp>("POST", `/records/${id}/revoke`),
    share: (id: string) => request<ShareResp>("POST", `/records/${id}/share`),
    /** record-JWT bearer (UNAUTHENTICATED by operator session) */
    getRecord: (id: string, recordJwt: string) =>
      request<Record<string, unknown>>("GET", `/records/${id}`, undefined, "bearer", recordJwt),

    // ---- profile / dog-tag issuance (operator session, custody unlocked) ----
    startProfileIssue: (body: ProfileIssueStartReq) =>
      request<ProfileIssueStartResp>("POST", "/profiles/issue/session/start", body),
    /** GET /profiles/issue/session/{sessionId} — operator-gated status poll (pending → bound). */
    profileIssueStatus: (sessionId: string) =>
      request<ProfileIssueStatusResp>("GET", `/profiles/issue/session/${sessionId}`),

    // ---- issuer signers ----
    issuerSigners: () => request<IssuerSignersResp>("GET", "/issuer/signers"),

    // ---- import ----
    importPull: (body: ImportPullReq) => request<ImportPullResp>("POST", "/import/pull", body),

    // ---- verify ----
    verifyCredential: (body: VerifyCredentialReq) =>
      request<VerifyCredentialResp>("POST", "/verify/credential", body),
    verifySessionStart: (body: VerifySessionStartReq) =>
      request<VerifySessionStartResp>("POST", "/verify/session/start", body),
    /** GET /verify/session/{id} — operator-gated status poll (pending → recorded). */
    verifySessionStatus: (id: string) =>
      request<VerifySessionStatusResp>("GET", `/verify/session/${id}`),
    /** GET /verify/history — operator-gated verification audit trail, most recent first. */
    verificationHistory: () => request<VerificationHistoryResp>("GET", "/verify/history"),
    verifyConsentSubmit: (body: VerifyConsentSubmitReq) =>
      request<VerifyConsentSubmitResp>("POST", "/verify/consent/submit", body),

    // ---- central: issuer-application apply (whitelist apply relays here) ----
    applyForWhitelist: (body: IssuerApplicationReq) =>
      request<IssuerApplicationResp>("POST", "/v1/issuer-applications", body, "none", undefined, "central"),
  };
}

export type ApiClient = ReturnType<typeof createApiClient>;
