/**
 * Wire types mirroring the vet backend (`stacks/vet/api/src/routes.rs`) and the central
 * admin API (`stacks/admin/api/src/routes.rs`) JSON contracts. Field names match the Rust
 * serde renames exactly.
 */

export type SigningMode = "wallet" | "backend";
export type VerifyMode = "normal" | "zk";
export type RecordStatus = "prepared" | "confirming" | "issued" | "revoked" | "expired";

// ---- auth ----
export interface LoginResp {
  token: string;
}

// ---- genesis / custody (admin router) ----
export interface GenesisStartResp {
  words: string[];
  challengeIndices: number[];
}
export interface GenesisConfirmReq {
  /** the words re-typed at challengeIndices, in challenge-index order */
  words: string[];
  passphrase: string;
}
export interface GenesisConfirmResp {
  address: string;
}
export interface UnlockReq {
  passphrase: string;
}
export interface AccountInfo {
  index: number;
  address: string;
  label: string;
}
export interface UnlockResp {
  unlocked: boolean;
  accounts: AccountInfo[];
}
export interface AccountsReq {
  label: string;
}
export interface AccountsResp {
  index: number;
  address: string;
}

// ---- settings ----
export interface SigningModeResp {
  signingMode: SigningMode;
}

// ---- credentials ----
export interface PrepareReq {
  recordType: string;
  dogTagId: string;
  fields: Record<string, unknown>;
}
export interface UnsignedTx {
  to: string;
  data: string;
  value: number;
  chainId: number;
}
/** wallet mode returns an unsignedTx; backend mode returns txHash + signerAddress. */
export interface PrepareResp {
  recordId: string;
  merkleRoot: string;
  targetHash?: string;
  proof?: string[];
  unsignedTx?: UnsignedTx;
  txHash?: string;
  signerAddress?: string;
  mode?: SigningMode;
}
export interface ConfirmReq {
  recordId: string;
  txHash: string;
}
export interface ConfirmResp {
  recordId: string;
  status: "issued";
}

// ---- records ----
export interface RevokeResp {
  recordId: string;
  status: "revoked";
  txHash: string;
  blockNumber?: number | null;
}
export interface ShareResp {
  qrUrl: string;
}

/**
 * A persisted credential record as stored in the backend's OWN database (`GET /records`). Field names
 * mirror the Rust `store::Record` serde defaults (snake_case). Bundles the credential data with its
 * IMMUTABLE on-chain proof (tx hash, block number, contract/issuer address, explorer link). The
 * source of truth for the operator's records surface — NOT a browser cache.
 */
export interface DbRecord {
  record_id: string;
  record_type: string;
  dog_tag_id: string;
  root: string;
  issuer_addr: string;
  status: RecordStatus;
  tx_hash?: string | null;
  confirmed_tx_hash?: string | null;
  block_number?: number | null;
  /** ready-to-click block-explorer link for the anchoring tx. */
  explorer_url?: string | null;
  created_at?: number;
  updated_at?: number;
  /** off-chain, operator-editable metadata. */
  label?: string | null;
  notes?: string | null;
  /** on-chain revocation proof (set once revoked). */
  revoked_tx_hash?: string | null;
  revoked_block_number?: number | null;
  revoke_explorer_url?: string | null;
  invalidated_at?: number | null;
  invalidation_reason?: string | null;
  signer_address?: string | null;
  signing_mode?: string | null;
}
export interface RecordsListResp {
  records: DbRecord[];
}

// ---- traceability portal (govarch PR-5): scoped on-chain activity joined to own DB records --------

export type TraceEventType =
  | "issuerCreated"
  | "rootRegistered"
  | "whitelisted"
  | "delisted"
  | "rootIssued"
  | "rootRevoked"
  | "verified";

/** The local DB record/verification/mint joined to an on-chain event (this operator's own). */
export interface TraceLocalJoin {
  /** "mint" = a dog-tag issuance (ProfileIssueSession) joined by its anchored profile root. */
  kind: "issuance" | "verification" | "mint";
  recordId?: string;
  recordType?: string;
  dogTagId?: string;
  status?: string;
  label?: string | null;
  notes?: string | null;
  /** verification + mint joins carry a session handle instead of a record id. */
  sessionId?: string;
  purpose?: string;
  mode?: string;
}

/** One non-PII on-chain oversight event, as re-served (and joined) by the `/trace` feed. */
export interface TraceEvent {
  id: string;
  type: TraceEventType;
  contract?: string;
  blockNumber?: number;
  txHash?: string;
  blockTimestamp?: number;
  finality?: "finalized" | "pending";
  actor?: string;
  clone?: string;
  recordType?: string;
  root?: string;
  dogTagId?: string;
  /** `verified` events (owner-blind): the hashed purpose key the consent was proven for. */
  purpose?: string;
  /** `verified` events: the unlinkable consent nullifier. */
  nullifier?: string;
  /** `verified` events: the consent window's proof-bound deadline (unix seconds). */
  deadline?: number;
  name?: string;
  actorName?: string;
  cloneName?: string;
  txUrl?: string;
  /** The joined local record/verification, or null when there is no matching local record. */
  local?: TraceLocalJoin | null;
}

/** Narrowing filters for the scoped `/trace/activity` feed (can only shrink, never widen, scope). */
export interface TraceQuery {
  type?: TraceEventType;
  signer?: string;
  issuer?: string;
  recordType?: string;
  root?: string;
  dogTagId?: string;
  finality?: "finalized" | "pending";
  since?: number;
  until?: number;
  limit?: number;
  offset?: number;
}

export interface TraceActivityResp {
  events: TraceEvent[];
  total?: number;
  limit?: number;
  offset?: number;
  /** events admitted by the local scope gate (== events.length). */
  inScope?: number;
  /** events joined to one of this operator's own local records. */
  matched?: number;
  /** events the indexer returned that the local scope gate rejected (0 when scoped correctly). */
  droppedOutOfScope?: number;
  scope?: { label?: string; unscoped?: boolean };
  localScope?: { signers: number; clones: number };
}

export interface TraceStatsResp {
  rootIssued?: number;
  rootRevoked?: number;
  activeCredentials?: number;
  verifications?: number;
  scope?: { label?: string; unscoped?: boolean };
  /** this operator's own off-chain record/verification/mint counts. */
  local?: { records: number; verifications: number; dogTagsMinted?: number };
  [k: string]: unknown;
}
/** PATCH /records/:id — OFF-CHAIN metadata only. On-chain-derived fields are rejected by the backend. */
export interface UpdateRecordReq {
  label?: string | null;
  notes?: string | null;
  /** only "expired" is a permitted off-chain transition (validity lapse, no chain tx). */
  status?: "expired";
  reason?: string;
}

// ---- issuer signers (whitelist matrix) ----
export interface WhitelistRow {
  recordType: string;
  address: string;
  whitelisted: boolean;
}
export interface IssuerSignersResp {
  activeSigner: string;
  matrix: WhitelistRow[];
}

// ---- import ----
export interface ImportPullReq {
  userApiBase: string;
  userJwt: string;
  recordRef: string;
}
export interface ImportPullResp {
  imported: boolean;
  verdict: unknown;
}

// ---- verify ----
export interface VerifyCredentialReq {
  /** WrappedDoc JSON as produced by a DogTag issuer. */
  wrappedDoc: Record<string, unknown>;
  /** Optional DogTagIssuer clone override; defaults to wrappedDoc.issuer.documentStore. */
  issuerAddr?: string;
  /** Optional issuer signer address for the IssuerRegistry whitelist pillar. */
  signerAddr?: string;
}
export interface VerifyCredentialResp {
  verdict: boolean;
  /** Current direct-check status from integrity + chain reads. */
  status: "valid" | "revoked" | "not_issued" | "integrity_failed" | "invalid";
  recordType: string;
  root: string;
  recomputedRoot: string;
  issuerAddr: string;
  signerAddr?: string | null;
  /** Unix seconds as a decimal string from DogTagIssuer.issuedAt(root). */
  issuedAt: string;
  checkedAt: number;
  fragments: {
    integrity: boolean;
    onchain: boolean;
    issued: boolean;
    revoked: boolean;
    issuerWhitelisted?: boolean | null;
  };
}
export interface VerifySessionStartReq {
  purpose: string;
  recordType: string;
  mode?: VerifyMode;
}
export interface VerifySessionStartResp {
  qrUrl: string;
  sessionId: string;
}
export interface VerifyConsentSubmitReq {
  sessionId: string;
  consent: Record<string, unknown>;
  sig: string;
  mode?: VerifyMode;
  disclosedDoc?: Record<string, unknown>;
}
export interface VerifyConsentSubmitResp {
  recorded: boolean;
  txHash?: string;
  mode?: VerifyMode;
}
/** GET /verify/session/{id} — operator-gated status read used by VerifyFlow's poller. */
export interface VerifySessionStatusResp {
  status: string;
  mode?: VerifyMode;
  txHash?: string | null;
  nullifier?: string | null;
}
export interface VerificationHistoryItem {
  sessionId: string;
  relayer: string;
  purpose: string;
  recordType: string;
  mode: VerifyMode | string;
  status: "pending" | "recording" | "recorded" | "error" | string;
  txHash?: string | null;
  explorerUrl?: string | null;
  nullifier?: string | null;
  createdAt: number;
  updatedAt: number;
}
export interface VerificationHistoryResp {
  verifications: VerificationHistoryItem[];
}

// ---- profile / dog-tag issuance (operator session) ----
/** Microchip standard accepted by the backend. */
export type MicrochipStandard = "ISO_11784_11785" | "OTHER";
export type PetSex = "male" | "female";
export type NeuterStatus = "intact" | "neutered" | "spayed";

export interface ProfileWeightEntry {
  unit: string;
  /** decimal string, e.g. "12.5" */
  value: string;
  measuredOn: string;
}
export interface ProfileMicrochip {
  code: string;
  standard: MicrochipStandard;
  implantDate?: string;
  bodyLocation?: string;
}
export interface ProfileOwnerIdentity {
  countryOfIdentification: string;
  identification: string;
  name: string;
}
export interface ProfilePet {
  /** required */
  name: string;
  species?: string;
  breedVbo?: string;
  breedLabel?: string;
  sex?: PetSex;
  neuterStatus?: NeuterStatus;
  dateOfBirth?: string;
  weightHistory?: ProfileWeightEntry[];
  microchip?: ProfileMicrochip;
}
/** POST /profiles/issue/session/start body. */
export interface ProfileIssueStartReq {
  ownerIdentity: ProfileOwnerIdentity;
  pet: ProfilePet;
}
/** POST /profiles/issue/session/start response. `qr` is the full <deployment_url>/p/<token> URL. */
export interface ProfileIssueStartResp {
  token: string;
  dogTagId: string;
  sessionId: string;
  qr: string;
}
/** GET /profiles/issue/session/{sessionId} response. */
export interface ProfileIssueStatusResp {
  status: "pending" | "bound";
  dogTagId: string;
  walletAddress?: string | null;
  root?: string | null;
  txHash?: string | null;
}

// ---- central: issuer applications (admin/api §4.3) ----
export interface CentralLicense {
  number: string;
  jurisdiction: string;
  expiry: string;
}
export interface IssuerApplicationReq {
  issuerEntityId: string;
  addresses: string[];
  recordTypes: string[];
  /**
   * Optional VERIFY:<purpose> whitelist purposes. Approval whitelists VERIFY:<purpose> per address
   * (in addition to the recordType issuance whitelist). e.g. ["grooming_intake", "boarding_intake"].
   */
  verifyPurposes?: string[];
  domain: string;
  documentStore: string;
  usdaNan?: string;
  license?: CentralLicense;
}
export interface IssuerApplicationResp {
  applicationId: string;
  status: "pending";
}

export interface ApiError extends Error {
  status: number;
  body?: unknown;
}

// ============================================================================================
// central admin API (`stacks/admin/api/src/routes.rs`) — used by the admin portal.
// Field names mirror the Rust serde renames exactly.
// ============================================================================================

// ---- admin auth ----
/** POST /v1/admin/login → { token } */
export interface AdminLoginResp extends LoginResp {
  /** custody already initialized (seal present) -> route to Unlock, not Genesis. */
  initialized?: boolean;
  /** custody already unlocked this session. */
  unlocked?: boolean;
}

// ---- business registry (§4.2) ----
export interface BusinessGeo {
  lat: number;
  lng: number;
}
/** GET /v1/businesses item (non-personal fields only; never the HMAC secret). */
export interface CentralBusiness {
  businessId: string;
  type: string;
  name: string;
  geo: BusinessGeo;
  services: string[];
  apiBaseUrl: string;
  domain: string;
  documentStores: string[];
  hmacKeyId: string;
}
export interface BusinessesResp {
  businesses: CentralBusiness[];
}
/** GET /v1/businesses query filters. */
export interface BusinessesQuery {
  type?: string;
  /** "lat,lng" */
  near?: string;
  /** km */
  radius?: number;
}
export interface RegisterBusinessReq {
  type: string;
  name: string;
  lat: number;
  lng: number;
  services?: string[];
  apiBaseUrl: string;
  domain: string;
  documentStores?: string[];
}
/** POST /v1/businesses → the HMAC secret is returned ONCE at registration. */
export interface RegisterBusinessResp {
  businessId: string;
  hmacKeyId: string;
  hmacSecret: string;
}

// ---- issuer applications queue (§4.3) ----
export type IssuerApplicationStatus =
  | "pending"
  | "approved"
  | "rejected"
  | "delisted";
/** GET /v1/issuer-applications item (multi-address × multi-recordType per entity). */
export interface IssuerApplicationListItem {
  applicationId: string;
  issuerEntityId: string;
  addresses: string[];
  recordTypes: string[];
  /** VERIFY:<purpose> purposes whitelisted on approval (may be empty/absent for issuer-only apps). */
  verifyPurposes?: string[];
  domain: string;
  status: IssuerApplicationStatus;
}
export interface IssuerApplicationsResp {
  applications: IssuerApplicationListItem[];
}
/** POST /v1/issuer-applications/{id}/approve → on-chain whitelistFor per (address,recordType). */
export interface ApproveApplicationResp {
  status: "approved";
  whitelistTxs: string[];
  /**
   * True when the application is a dog-tag issuer (recordTypes include DOG_PROFILE): approval ALSO
   * grants DogTagSBT.ISSUER_ROLE so the signer can mint dog tags. False for groomers / verify-only.
   */
  issuerRoleGranted: boolean;
  /** The grantRole(ISSUER) tx hash, when a grant was broadcast (absent if already held / not a dog-tag issuer). */
  issuerRoleTxHash?: string | null;
}
export interface RejectApplicationResp {
  status: "rejected";
}
export interface DelistApplicationResp {
  status: "delisted";
  delistTxs: string[];
}

// ---- direct whitelist management (PR-E) ----
// `GovernanceDisposition` (the shared GovernanceAction outcome type) is defined once in the
// control-plane block below; the whitelist responses reuse it.
/**
 * POST /v1/admin/whitelist/{grant,revoke} body — whitelist/delist a (signer, capability) pair
 * directly, decoupled from the issuer-application queue. At least one of `recordType` /
 * `verifyPurposes` must be present. `recordType` is a label ("VACCINATION") or an explicit
 * `0x`+64-hex key; `verifyPurposes` are VERIFY:<purpose> capabilities.
 */
export interface WhitelistActionReq {
  signer: string;
  recordType?: string;
  verifyPurposes?: string[];
}
/** POST /v1/admin/whitelist/grant response: one disposition per whitelisted capability. */
export interface WhitelistGrantResp {
  signer: string;
  recordType?: string | null;
  actions: GovernanceDisposition[];
  /**
   * DOG_PROFILE grants ALSO grant DogTagSBT.ISSUER_ROLE (mint rights): the disposition of that grant,
   * `{ status: "alreadyHeld" }` when the signer already had it, or null for non-DOG_PROFILE grants.
   */
  issuerRole?: GovernanceDisposition | { status: "alreadyHeld" } | null;
  /** False when NOTHING reached the chain: on-chain state is unchanged. */
  executed?: boolean;
  /** Set only when `executed` is false: a plain statement that nothing was broadcast. */
  warning?: string | null;
}
/** POST /v1/admin/whitelist/revoke response: one disposition per delisted capability. */
export interface WhitelistRevokeResp {
  signer: string;
  recordType?: string | null;
  actions: GovernanceDisposition[];
  /** False when NOTHING reached the chain: on-chain state is unchanged. */
  executed?: boolean;
  /** Set only when `executed` is false: a plain statement that nothing was broadcast. */
  warning?: string | null;
}

// ---- appointments (§4.4) ----
export interface CentralAppointment {
  id: string;
  businessId: string;
  dogTagId: string;
  slot: string;
  rev: number;
  state: string;
  updatedAt: number;
}
export interface AppointmentsResp {
  appointments: CentralAppointment[];
}

// ---- consents (§4.5) ----
export interface CentralConsent {
  consentId: string;
  purpose: string;
  lawfulBasis: string;
  grantedAt: number;
  withdrawn: boolean;
}
export interface ConsentsResp {
  consents: CentralConsent[];
}

// ---- control plane: factory deploys + governance authority (PR-A backend / PR-C UI) ----
/** POST /v1/admin/factory/predict — deterministic clone-address preview (no tx). */
export interface PredictIssuerReq {
  /** human label (`VACCINATION`) hashed server-side, or an already-hashed `0x`+64-hex key. */
  recordType: string;
  /** the `business` salt component; defaults server-side to the hosted signer (single-authority topology). */
  business?: string;
}
export interface PredictIssuerResp {
  /** the deterministic CREATE2 clone address for (recordType, business). */
  predicted: string;
  /** the keccak256(recordType) salt key actually used. */
  recordTypeKey: string;
  /** the resolved `business` salt component (echoed so the caller sees the default). */
  business: string;
  /** the factory the clone deploys from. */
  factory: string;
}
/** POST /v1/admin/factory/issuers — deploy a clone (routed through the GovernanceAction layer). */
export interface CreateIssuerReq {
  name: string;
  recordType: string;
  business?: string;
}
/**
 * The outcome of dispatching the deploy through the key-holder-agnostic `GovernanceAction` layer.
 * `executed` — the hosted signer IS the factory owner, so `createIssuer` was signed and broadcast.
 * `proposed` — factory ownership sits with the governance signer (post Phase-2), so nothing was
 * broadcast; the `{target, calldata}` payload is handed to that holder to execute out-of-band.
 */
export type GovernanceDisposition =
  | { disposition: "executed"; txHash: string; holder: string; summary: string }
  | {
      disposition: "proposed";
      holder: string | null;
      /**
       * The hosted signer that was CHECKED and found NOT to hold the authority. Distinguishes "the
       * authority legitimately lives on the governance signer" from "this stack booted the wrong key",
       * which are otherwise identical: both just say `proposed`.
       */
      hostedSigner?: string | null;
      target: string;
      calldata: string;
      authority: string;
      summary: string;
    };
export interface CreateIssuerResp {
  predicted: string;
  recordTypeKey: string;
  business: string;
  result: GovernanceDisposition;
}
/** One authority slot in the live on-chain authority map (GET /v1/admin/governance/authority). */
export interface AuthoritySlot {
  target: string;
  /** the current holder if resolvable (Owner + DEFAULT_ADMIN), else null (ordinary roles). */
  owner?: string | null;
  holder?: string | null;
  pendingOwner?: string | null;
  pendingTransfer?: { newAdmin: string; acceptSchedule: number } | null;
  /** whether the hosted operator key holds this authority (null when unknown/unreachable). */
  heldByHosted: boolean | null;
  role?: string;
  capability: string;
}
export interface GovernanceAuthorityResp {
  /** the hosted operator signer address (the key the control plane signs with). */
  hostedSigner: string | null;
  chainId: number;
  factoryOwner: AuthoritySlot;
  whitelistAdmin: AuthoritySlot;
  defaultAdmin: AuthoritySlot;
}
/** One deployed `DogTagIssuer` clone with cross-issuer issuance counters (GET /v1/admin/activity/issuers). */
export interface IssuerCloneStat {
  clone: string;
  /** name from the IssuerCreated event (may be null). */
  name?: string | null;
  /** authoritative business name from the admin signer→business directory (may be null). */
  cloneName?: string | null;
  recordType?: string | null;
  issued: number;
  revoked: number;
  active: number;
}
export interface AdminActivityIssuersResp {
  issuers: IssuerCloneStat[];
}

// ============================================================================================
// central admin API - oversight-indexer consumption (PR-B data layer, rendered by PR-D).
// The UNSCOPED, non-PII cross-issuer activity surface: `/v1/admin/activity[/stats|/status|/issuers]`
// and `/v1/admin/directory`. Field names mirror the indexer's serde renames, re-enriched by the admin
// backend with its authoritative signer→business names (`actorName`/`cloneName`).
// ============================================================================================

/** The seven on-chain event kinds the oversight indexer surfaces (stable `?type=` wire tokens). */
export type ActivityEventType =
  | "issuerCreated"
  | "rootRegistered"
  | "whitelisted"
  | "delisted"
  | "rootIssued"
  | "rootRevoked"
  | "verified";
/** Finality lifecycle: `finalized` (settled/immutable) vs `pending` (still reorg-able). */
export type ActivityFinality = "finalized" | "pending";

/**
 * One flattened non-PII on-chain event (`GET /v1/admin/activity`). Hex fields are lowercase `0x…`.
 * `actorName`/`cloneName` are the admin backend's authoritative business names for `actor`/`clone`
 * (present only when the signer/clone resolves in the signer→business directory). `txUrl` is the
 * indexer-provided explorer link.
 */
export interface ActivityEvent {
  id: string;
  type: ActivityEventType;
  contract: string;
  blockNumber: number;
  blockHash?: string;
  txHash: string;
  logIndex?: number;
  blockTimestamp: number;
  finality: ActivityFinality;
  /** the acting signer (`by`/`signer`/`relayer`); absent for factory events. */
  actor?: string;
  /** the `DogTagIssuer` clone the event pertains to; absent for registry-level events. */
  clone?: string;
  recordType?: string | null;
  root?: string | null;
  dogTagId?: string | null;
  /** issuer display name carried on `IssuerCreated`. */
  name?: string | null;
  /** admin-directory business name for `actor`, when resolvable. */
  actorName?: string | null;
  /** admin-directory business name for `clone`, when resolvable. */
  cloneName?: string | null;
  /** ready-to-click explorer link for the anchoring tx (from the indexer). */
  txUrl?: string | null;
}
/** GET /v1/admin/activity - newest-first, paginated, unscoped feed. */
export interface ActivityResp {
  events: ActivityEvent[];
  total: number;
  limit: number;
  offset: number;
  scope?: { label: string; unscoped: boolean };
}
/** GET /v1/admin/activity query filters - every field only NARROWS the unscoped feed. */
export interface ActivityQuery {
  type?: ActivityEventType;
  signer?: string;
  issuer?: string;
  recordType?: string;
  root?: string;
  dogTagId?: string;
  finality?: ActivityFinality;
  /** inclusive lower/upper `blockTimestamp` bounds (unix seconds). */
  since?: number;
  until?: number;
  limit?: number;
  offset?: number;
}
/** GET /v1/admin/activity/stats - cross-issuer aggregate counters (dashboard fuel). */
export interface ActivityStats {
  totalEvents: number;
  finalized: number;
  pending: number;
  rootIssued: number;
  rootRevoked: number;
  activeCredentials: number;
  verifications: number;
  whitelisted: number;
  delisted: number;
  clones: number;
  signers: number;
  scope?: { label: string; unscoped: boolean };
}
/** GET /v1/admin/activity/status - indexer progress + finality watermark (chain-health card). */
export interface IndexerStatus {
  chainId: number;
  /** current chain head block (null when the RPC is unreachable). */
  headBlock: number | null;
  /** latest block the indexer treats as finalized. */
  finalizedBlock: number | null;
  /** how the finalized watermark was derived. */
  finalitySource: "finalized-tag" | "confirmations-fallback" | "unknown";
  /** the furthest block the scanner has settled into the index. */
  lastFinalizedIndexed: number | null;
  /** head − lastFinalizedIndexed (indexing lag in blocks; null when unknown). */
  lag: number | null;
  confirmations: number;
  scope?: { label: string; unscoped: boolean };
}
/** GET /v1/admin/activity/issuers - per-clone issued/revoked/active counts. */
export interface ActivityIssuer {
  clone: string;
  name?: string | null;
  cloneName?: string | null;
  recordType?: string | null;
  issued: number;
  revoked: number;
  active: number;
}
export interface ActivityIssuersResp {
  issuers: ActivityIssuer[];
  scope?: { label: string };
}
/** One signer→business directory entry (`GET /v1/admin/directory`). Non-PII: business identity only. */
export interface DirectorySigner {
  signer: string;
  business?: string;
  businessId?: string;
  entity: string;
  recordTypes: string[];
  verifyPurposes: string[];
  domain: string;
  applicationId: string;
  status: IssuerApplicationStatus;
}
export interface DirectoryResp {
  signers: DirectorySigner[];
  total: number;
}
/** GET /v1/admin/governance/authority - the live on-chain authority map (PR-A; chain-health card). */
export interface AuthorityRole {
  target?: string;
  role?: string;
  owner?: string | null;
  holder?: string | null;
  pendingOwner?: string | null;
  pendingTransfer?: { newAdmin: string; acceptSchedule: number } | null;
  heldByHosted?: boolean | null;
  capability?: string;
}
export interface GovernanceAuthority {
  hostedSigner: string | null;
  chainId: number;
  factoryOwner: AuthorityRole;
  whitelistAdmin: AuthorityRole;
  defaultAdmin: AuthorityRole;
}

// ---- import verdict (3 authenticity pillars + contextual ownership) ----
export type FragmentState = "VALID" | "INVALID" | "ERROR" | "NOT_APPLICABLE";
/** Shape of ImportPullResp.verdict from the vet/groomer backend `verify::verdict_json`. */
export interface ImportVerdict {
  valid: boolean;
  integrity: FragmentState;
  issuance: FragmentState;
  identity: FragmentState;
  ownership: FragmentState;
}
