import type {
  AccountsReq,
  AccountsResp,
  ApiError,
  AppointmentInput,
  AppointmentListQuery,
  AppointmentShareResp,
  ClientInput,
  ClientListQuery,
  ConfirmReq,
  ConfirmResp,
  CrmAppointment,
  CrmClient,
  CrmPage,
  CrmPet,
  CrmVerification,
  GenesisConfirmReq,
  GenesisConfirmResp,
  GenesisStartResp,
  HealthResp,
  IcsFeedResp,
  IcsImportEventInput,
  IcsImportResp,
  ImportPullReq,
  ImportPullResp,
  IssuerApplicationReq,
  IssuerApplicationResp,
  IssuerSignersResp,
  LoginResp,
  AdminLoginResp,
  MicrochipCheck,
  PetCreateInput,
  PetCredentialsResp,
  PetListQuery,
  PetPatchInput,
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
  VerificationListQuery,
  VerifyCredentialReq,
  VerifyCredentialResp,
  VerifySessionStartReq,
  VerifySessionStartResp,
  VerifySessionStatusResp,
} from "./types";
import { isCustodyLockedError, isWrongPassphraseError } from "../custody/lock";
// The wire shape of the clone's own issuance list. Declared in `signers/roster.ts` beside the rules
// that decide what each row may CLAIM, so a consumer cannot get the type without the decisions.
import type { IssuanceAllowedResponse } from "../signers/roster";

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
   * `isCustodyLockedError`). The host raises its point-of-need unlock prompt and resolves TRUE once
   * the seal is open, at which point this client REPLAYS the refused request exactly once, so the
   * operator's action continues instead of dying in an error toast. Resolving false (or omitting the
   * handler) rethrows the original refusal.
   *
   * Replay is safe because the backends check `is_unlocked()` at the TOP of each handler, before any
   * store or chain write, so a "not unlocked" refusal means nothing happened. Purely a client-side
   * signal — the backend's locked state and its gates are untouched.
   */
  onCustodyLocked?: () => boolean | Promise<boolean>;
}

type TokenKind = "operator" | "admin" | "bearer" | "none";

/**
 * A 401 that REJECTS A SUBMITTED CREDENTIAL rather than reporting a dead session, keyed on the
 * backend's message. `/admin/unlock` answers a wrong passphrase with 401; clearing the just-issued
 * admin token there would replace the unlock page's inline "wrong passphrase" with a bogus "session
 * expired". Exempting the whole path would go too far — the same route's admin gate raises `missing
 * admin session` / `invalid admin session` with the same status, and those ARE dead sessions.
 */
function isCredentialRejection(err: unknown): boolean {
  return isWrongPassphraseError(err);
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
    /** Internal: set on the single post-unlock replay so a still-locked backend cannot loop. */
    isRetry = false,
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
      const e = makeError(res.status, parsed);
      // Stale-session handling: a 401 on a token-bearing call means the persisted session was
      // invalidated (e.g. the backend restarted its in-memory session store). Clear it so the UI
      // routes back to login instead of replaying a dead token.
      if (
        res.status === 401 &&
        (tokenKind === "operator" || tokenKind === "admin") &&
        !explicitToken &&
        !isCredentialRejection(e)
      ) {
        opts.onUnauthorized?.(tokenKind);
      }
      // Locked-custody handling: a restart drops the decrypted seed, so any custody-backed call
      // starts failing with "not unlocked". Surface it centrally, let the host unlock in place, then
      // replay the request ONCE so the operator's action completes and their form survives.
      if (isCustodyLockedError(e) && !isRetry && opts.onCustodyLocked) {
        const unlocked = await opts.onCustodyLocked();
        if (unlocked) return request<T>(method, path, body, tokenKind, explicitToken, rootBase, true);
      }
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

  /**
   * Serialize list filters into a `?a=b&…` query string, DROPPING absent/blank values so a cleared
   * UI filter is genuinely "no filter" rather than a filter on the empty string. Filtering and
   * paging are the server's job — the browser never pulls a collection to narrow it locally.
   */
  function queryString(params: Record<string, string | number | undefined | null>): string {
    const sp = new URLSearchParams();
    for (const [k, v] of Object.entries(params)) {
      if (v === undefined || v === null || v === "") continue;
      sp.set(k, String(v));
    }
    const s = sp.toString();
    return s ? `?${s}` : "";
  }

  /** Build a `?a=b&…` query string from a TraceQuery, skipping unset/blank fields. */
  function traceQs(q?: TraceQuery): string {
    return q ? queryString(q as Record<string, string | number | undefined | null>) : "";
  }

  return {
    base,
    central,

    // ---- health ----
    /**
     * GET /health — unauthenticated. Besides liveness, an issuance-enabled backend reports whether
     * the dog-tag ANCHOR contracts are configured (`dogTagIssuance`), which is what lets the
     * Register pet screen refuse IN PLACE instead of allocating a tag for an issuance the backend
     * already knows it cannot complete.
     */
    health: () => request<HealthResp>("GET", "/health", undefined, "none"),

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

    // ---- the clone's own issuance list (LAYER 2 of the two-layer issuance requirement) ----
    /**
     * GET /issuer/issuance-allowed — for each contract this deployment anchors through: who owns
     * it, which addresses its own issuance list has an opinion about, and whether THIS shop's
     * custody signer is among them. Operator-gated, read-only.
     *
     * There is deliberately no write counterpart. `setIssuanceAllowed` admits only from the
     * contract's `owner()`, which this backend is not — its custody signer is the address that
     * needs admitting — and an operator session proves "staff of this shop", never "owner of this
     * contract". The write is a wallet transaction; see `IssuerSignersPanel`.
     */
    issuanceAllowed: () =>
      request<IssuanceAllowedResponse>("GET", "/issuer/issuance-allowed"),

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

    /**
     * Re-arm a FAILED issuance with a fresh one-time QR: same session, same dogTagId, same
     * identity salts — the only path that can complete a stranded (anchored, unminted) root,
     * because a fresh session derives a DIFFERENT root from its new dogTagId.
     */
    retryProfileIssue: (sessionId: string) =>
      request<ProfileIssueStartResp>("POST", `/profiles/issue/session/${sessionId}/retry`),
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

    // ---- shop CRM: clients (operator session) ----
    listClients: (q: ClientListQuery = {}) =>
      request<CrmPage<CrmClient>>("GET", `/clients${queryString({ ...q })}`),
    getClient: (id: string) => request<CrmClient>("GET", `/clients/${id}`),
    createClient: (body: ClientInput) => request<CrmClient>("POST", "/clients", body),
    updateClient: (id: string, body: ClientInput) => request<CrmClient>("PUT", `/clients/${id}`, body),
    deleteClient: (id: string) => request<{ deleted: boolean }>("DELETE", `/clients/${id}`),

    // ---- shop CRM: pets (a collection of their own; stored inside their owner) ----
    listPets: (q: PetListQuery = {}) =>
      request<CrmPage<CrmPet>>("GET", `/pets${queryString({ ...q })}`),
    getPet: (id: string) => request<CrmPet>("GET", `/pets/${id}`),
    createPet: (body: PetCreateInput) => request<CrmPet>("POST", "/pets", body),
    /** PATCH semantics on a PUT: an omitted field is left as it is, never blanked. */
    updatePet: (id: string, body: PetPatchInput) => request<CrmPet>("PUT", `/pets/${id}`, body),
    deletePet: (id: string) => request<{ deleted: boolean }>("DELETE", `/pets/${id}`),
    /**
     * LINK a DogTag to this pet — writes the SHOP's own note of which tag the pet holds. Mints
     * nothing and writes nothing on chain.
     *
     * The response ALWAYS carries `microchipCheck`, in every state, so an absent key can never
     * be read as a check that passed. A mismatch is refused with 409 and the same structured verdict
     * rides in the error body ({@link MicrochipCheck}).
     */
    linkPetDogTag: (id: string, dogTagId: string) =>
      request<CrmPet & { microchipCheck: MicrochipCheck }>("POST", `/pets/${id}/dogtag`, {
        dogTagId,
      }),
    /**
     * UNLINK the DogTag from this pet. A LOCAL disassociation and nothing more: the credential stays
     * valid, stays on chain, and stays verifiable by everyone else. This is NOT revocation — see
     * `revoke` on the issuance surface, which is permanent and publicly visible.
     */
    unlinkPetDogTag: (id: string) => request<CrmPet>("DELETE", `/pets/${id}/dogtag`),
    /**
     * The credential document(s) this shop HOLDS for the pet's tag — what `POST /import/pull` stored.
     * Carries no verdict on purpose: validity is an on-chain fact that changes after the import, so
     * the caller re-checks the chain rather than trusting a frozen answer.
     */
    petCredentials: (id: string) => request<PetCredentialsResp>("GET", `/pets/${id}/credentials`),

    // ---- shop CRM: appointments ----
    listAppointments: (q: AppointmentListQuery = {}) =>
      request<CrmPage<CrmAppointment>>("GET", `/appointments${queryString({ ...q })}`),
    /** the single-appointment read also carries this appointment's verifications */
    getAppointment: (id: string) => request<CrmAppointment>("GET", `/appointments/${id}`),
    createAppointment: (body: AppointmentInput) =>
      request<CrmAppointment>("POST", "/appointments", body),
    updateAppointment: (id: string, body: AppointmentInput) =>
      request<CrmAppointment>("PUT", `/appointments/${id}`, body),
    deleteAppointment: (id: string) =>
      request<{ deleted: boolean }>("DELETE", `/appointments/${id}`),
    /**
     * POST /appointments/{id}/share — mint the CLIENT handoff for one booking: a link/QR resolving
     * to that appointment alone, from which the client adds it to their own calendar.
     *
     * The response's `qrUrl` is `null` when this deployment has no base a client's phone could
     * reach; render no QR and show `qrUnavailableReason` rather than building one from anything
     * else. See `AppointmentShareResp`.
     */
    shareAppointment: (id: string) =>
      request<AppointmentShareResp>("POST", `/appointments/${id}/share`),

    // ---- `.ics` calendar interop: subscription feed + import ----
    /** GET /calendar/feed — is a subscribable `.ics` feed published, and under which secret. */
    getIcsFeed: () => request<IcsFeedResp>("GET", "/calendar/feed"),
    /**
     * POST /calendar/feed/rotate — publish a feed, or replace an existing one's secret.
     *
     * Rotation is also the re-publish half of revocation: the previous URL stops working the instant
     * this returns, so every existing subscriber must be re-pointed at the new one.
     */
    rotateIcsFeed: () => request<IcsFeedResp>("POST", "/calendar/feed/rotate"),
    /** DELETE /calendar/feed — take the feed offline. The URL 404s immediately. */
    revokeIcsFeed: () => request<IcsFeedResp>("DELETE", "/calendar/feed"),
    /**
     * POST /calendar/import — create/update bookings from a parsed `.ics`.
     *
     * `dryRun` reports exactly what would happen and writes nothing. Events are deduped by their
     * source `UID`, so re-importing the same file updates rather than duplicating.
     */
    importIcsEvents: (events: IcsImportEventInput[], dryRun = false) =>
      request<IcsImportResp>(
        "POST",
        `/calendar/import${dryRun ? "?dryRun=1" : ""}`,
        { events },
      ),

    // ---- shop CRM: verification history ("All verifications") ----
    listVerifications: (q: VerificationListQuery = {}) =>
      request<CrmPage<CrmVerification>>("GET", `/verifications${queryString({ ...q })}`),
    getVerification: (id: string) => request<CrmVerification>("GET", `/verifications/${id}`),

    // ---- central: issuer-application apply (whitelist apply relays here) ----
    applyForWhitelist: (body: IssuerApplicationReq) =>
      request<IssuerApplicationResp>("POST", "/v1/issuer-applications", body, "none", undefined, "central"),
  };
}

export type ApiClient = ReturnType<typeof createApiClient>;
