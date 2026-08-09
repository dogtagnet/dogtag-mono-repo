/**
 * Wire types mirroring the vet backend (`stacks/vet/api/src/routes.rs`) and the central
 * admin API (`stacks/admin/api/src/routes.rs`) JSON contracts. Field names match the Rust
 * serde renames exactly.
 */

import type { ContactChannelRecord } from "../directory/channels";

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
  /** Immutable generation id (the lowercase factory address); join to status.watchedGenerations. */
  generation: string;
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
  /**
   * TRI-STATE on the wire, and `null` is not a neighbour of `false`.
   *
   * `true` / `false` are answers ABOUT THE SIGNER, read from the authority the clone itself names.
   * `null` says the read did not resolve - an unreachable RPC, an authority that answered in no
   * vocabulary this build knows - which is a fact about US and must never be rendered as "this signer
   * is not approved". It was a bare bool defaulting to `false` on any read failure, i.e. an operator's
   * own RPC blip shown to them as a permissions problem.
   *
   * Every renderer must branch on `null` FIRST; `null` is falsy, so a `w.whitelisted ? … : …` reads it
   * as a definite refusal. See {@link whitelistBadge}, which is the one place that decision is made.
   */
  whitelisted: boolean | null;
}
/**
 * `GET /issuer/signers` answers with TWO different shapes, and modelling only one of them is how a
 * caller ends up treating missing data as data.
 *
 * Custody UNLOCKED gives `{ activeSigner, matrix }`. Custody LOCKED short-circuits to
 * `{ signers: [] }` - no `activeSigner`, no `matrix`, and no signer of any kind. Every field is
 * therefore optional, and the locked shape is named rather than left to be inferred from a missing
 * key: a consumer must be able to tell "this shop has no signer" apart from "this portal could not
 * find out". Key on PRESENCE, never truthiness - `active_address()` can legitimately return `""`.
 */
export interface IssuerSignersResp {
  /** Present only when custody is unlocked. */
  activeSigner?: string;
  /** Present only when custody is unlocked; one row per (recordType, signer) pair. */
  matrix?: WhitelistRow[];
  /** The locked-custody shape. Always empty - it carries no signer to read. */
  signers?: string[];
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
  /**
   * Whether the imported credential's microchip matches the pet this shop has on file for its tag.
   *
   * Always present, in every state. A `mismatch` is refused with 409 and never reaches here —
   * and it does NOT alter `verdict`: the credential verified and is genuine, it simply describes a
   * different animal to the one on file. See {@link MicrochipCheck}.
   */
  microchipCheck: MicrochipCheck;
}

// ---- verify ----
export interface VerifyCredentialReq {
  /** WrappedDoc JSON as produced by a DogTag issuer. */
  wrappedDoc: Record<string, unknown>;
  /**
   * OPTIONAL *expected* DogTagIssuer clone. It can only TIGHTEN. The clone every read is made against
   * is the one the FACTORY names for this root, so this asserts an expectation and does NOT select a
   * contract - a caller able to nominate the contract that answers for a credential would reopen the
   * forgery the issuer pillar exists to close.
   */
  issuerAddr?: string;
  /**
   * OPTIONAL *expected* issuing signer. The whitelist pillar resolves its own signer from
   * `issuedBy(root)`; supplying this only adds the stricter assertion that the on-chain originator is
   * this address.
   */
  signerAddr?: string;
}
export interface VerifyCredentialResp {
  verdict: boolean;
  /**
   * Current direct-check status. The `issuer_*` arms exist so a failed or unevaluated issuer pillar
   * cannot leave this reading "valid" beside `verdict: false`.
   */
  status:
    | "valid"
    | "revoked"
    | "not_issued"
    | "integrity_failed"
    | "invalid"
    | "issuer_mismatch"
    | "issuer_not_whitelisted"
    | "issuer_unresolved";
  recordType: string;
  root: string;
  recomputedRoot: string;
  /** The clone the reads were actually made against (see `issuerResolution`). */
  issuerAddr: string;
  /**
   * How `issuerAddr` was arrived at. Anything other than `"resolved"` means it is the document's own
   * unverified claim, so the chain fragments below cannot carry a verdict on their own.
   */
  issuerResolution?: "resolved" | "noRecord" | "noFactoryConfigured";
  /** The document's claimed `issuer.documentStore`, echoed for comparison. */
  documentStore?: string;
  /** The signer the CHAIN recorded in `issuedBy[R]` - never one the caller supplied. */
  signerAddr?: string | null;
  /** The caller's assertions, echoed. Neither selects which contract answers. */
  expectedSignerAddr?: string | null;
  expectedIssuerAddr?: string | null;
  /** Unix seconds as a decimal string from DogTagIssuer.issuedAt(root). */
  issuedAt: string;
  checkedAt: number;
  fragments: {
    integrity: boolean;
    onchain: boolean;
    issued: boolean;
    revoked: boolean;
    /**
     * Was the on-chain originator authorised for this record type AT THE MOMENT this root was
     * anchored? Reconstructed from the governing registry's `Whitelisted`/`Delisted` logs, NOT from
     * `isWhitelistedFor` - delisting is forward-only (`DogTagIssuer.sol:82`; `adminRevoke` is the
     * retroactive lever), so a current-state read refuses every credential a since-rotated signer
     * ever issued.
     */
    issuerWhitelisted?: boolean | null;
    /**
     * Why `issuerWhitelisted` is what it is. A caller MUST be able to tell "not evaluated because this
     * verifier has no factory configured" from "evaluated and passed" - a pillar that never ran must
     * never read as one that succeeded, which is exactly what `null` alone used to imply.
     */
    issuerWhitelistState?:
      | "passed"
      | "failed"
      | "unresolved"
      | "unavailableNoFactoryConfigured";
    /** The envelope names a contract the chain disagrees with. */
    documentStoreDiffers?: boolean;
    /** The caller's expected-clone assertion disagrees with the factory-resolved clone. */
    expectedIssuerDiffers?: boolean;
    /**
     * What became of the caller's expected-clone assertion. `expectedIssuerDiffers` spells "held" and
     * "could not be checked" the same way (`false`), which is how a dropped check reads as a
     * satisfied one - this says which it was.
     */
    expectedIssuerState?: "notAsserted" | "matched" | "differs" | "notEvaluated";
    /**
     * What became of the caller's expected-signer assertion. It may only ever TIGHTEN: `differs` and
     * `unanchoredNotWhitelisted` are definite failures folded into the pillar, while
     * `unanchoredUnconfirmed` (no clone resolved, but the asserted address is whitelisted for the
     * claimed record type) deliberately promotes nothing - being whitelisted does not show that
     * address issued THIS root.
     *
     * This pair is the ONE place the CURRENT-state getter still answers: with no clone resolved there
     * is no trusted anchoring point to ask the historical question against, and taking one off the
     * document would let a forgery neutralise the last check standing. See the comment at the vet
     * handler for the bounded cost.
     */
    expectedSignerState?:
      | "notAsserted"
      | "matched"
      | "differs"
      | "unanchoredNotWhitelisted"
      | "unanchoredUnconfirmed";
  };
}
export interface VerifySessionStartReq {
  purpose: string;
  recordType: string;
  mode?: VerifyMode;
  /**
   * OPTIONAL shop appointment this verification is being performed for. Supplying it links the
   * resulting verification to that appointment and its client in the shop's verification history.
   * Omit for an ad-hoc verification.
   */
  appointmentId?: string;
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
/**
 * The `dogTagIssuance` block on an issuance-enabled backend's `GET /health`: whether the two
 * owner-hidden ANCHOR contracts (the DOG_PROFILE issuer clone and the DogTagSBTConsent) are
 * configured, and whether the vet signing key holds the SBT's mint role. `ready` is config facts
 * only (the addresses are set and well-formed, never that the contracts work); `mintRole` is the
 * ONE chain fact beside them — `mintCustodial` is `onlyRole(ISSUER_ROLE)` and a fresh SBT grants
 * it to nobody, so a fully configured stack can still be unable to mint a single tag (measured
 * live 2026-08-07). A groomer backend (no issuance surface) sends no block at all; consumers must
 * read absence — of the block, or of `mintRole` on an older backend — as could-not-check, never as
 * either verdict (`dogTagAnchorReadiness` encodes that rule).
 */
export interface DogTagIssuanceReadiness {
  ready: boolean;
  profileIssuerConfigured: boolean;
  sbtConsentConfigured: boolean;
  /** "held" | "missing" | "unknown"; absent on an older backend (= could-not-check). */
  mintRole?: "held" | "missing" | "unknown";
  /** The operator-vocabulary remedy (missing) or the could-not-check cause (unknown). */
  mintRoleDetail?: string | null;
}
/** GET /health — the unauthenticated liveness probe, plus the anchor-readiness block above. */
export interface HealthResp {
  status: string;
  dogTagIssuance?: DogTagIssuanceReadiness;
}

export interface ProfileIssueStartReq {
  ownerIdentity: ProfileOwnerIdentity;
  pet: ProfilePet;
}
/** POST /profiles/issue/session/start response. `qr` is the full <deployment_url>/p/<token> URL. */
/**
 * Whether the backend machine still answers at the address its QRs carry. The address is baked at
 * boot, so a machine that changed networks prints QRs no phone can reach — this is the backend's
 * own route-table check. `notSelfAddressed` carries the machine's CURRENT address (the remedy);
 * `unknown` (a hostname, or a failed lookup) carries why and must never be rendered as either
 * verdict — could-not-check is not an answer.
 */
export interface QrAddressCheck {
  host: string;
  check: "selfAddressed" | "notSelfAddressed" | "unknown";
  currentAddress?: string;
  detail?: string;
}
/**
 * The two-layer issue gate's answer at session start. A definite "this signer may not issue" never
 * appears here — that refuses the start itself (503) before anything is allocated. "unknown" means
 * one or both layers could not be checked; it WARNS and never refuses, and `detail` says what could
 * not be checked and where a later failure gets fixed.
 */
export interface SignerIssuanceCheck {
  state: "authorized" | "unknown";
  detail?: string;
}
export interface ProfileIssueStartResp {
  token: string;
  dogTagId: string;
  sessionId: string;
  qr: string;
  /** Seconds the QR waits to be scanned, counted from response receipt (never a wall-clock deadline). */
  ttlSecs?: number;
  qrAddress?: QrAddressCheck;
  signerIssuance?: SignerIssuanceCheck;
}
/** GET /profiles/issue/session/{sessionId} response. */
export interface ProfileIssueStatusResp {
  /** pending = waiting for a device; minting = a bind was accepted, chain writes in flight. */
  status: "pending" | "minting" | "bound" | "error";
  dogTagId: string;
  walletAddress?: string | null;
  root?: string | null;
  /** A REAL transaction hash (bound sessions) — a failed bind's reason lives in `error`. */
  txHash?: string | null;
  /** Unix seconds a device FIRST resolved the QR; null until any device picks it up. */
  resolvedAt?: number | null;
  /** The SERVER's remaining token life (meaningful while pending). 0 + pending = no bind can ever arrive. */
  tokenSecondsLeft?: number;
  qrAddress?: QrAddressCheck;
  /** WHY a failed bind failed, in the operator's vocabulary. Set iff `status === "error"`. */
  error?: string | null;
  /** WHERE it failed: "attestation" | "seal" | "issue" | "mint" | "verify" | "interrupted". */
  errorStage?: string | null;
}
/**
 * One row of GET /profiles/issue/sessions — the operator's route back to a failed issuance after a
 * page reload or a backend restart. A summary for recognition (pet, owner, tag id, what failed),
 * never the full session row.
 */
export interface ProfileIssueSessionRow {
  sessionId: string;
  dogTagId: string;
  status: "pending" | "minting" | "bound" | "error";
  /** Unix seconds the session was started. */
  createdAt: number;
  petName: string;
  ownerName: string;
  error?: string | null;
  errorStage?: string | null;
}
/** GET /profiles/issue/sessions response: recent sessions, newest first. */
export interface ProfileIssueSessionsResp {
  sessions: ProfileIssueSessionRow[];
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
/**
 * A business's own published contact channels.
 *
 * BUSINESS contact details - the number on the shop's door - deliberately distinct from an
 * `Owner`'s personal email. That is why they are served on the public `GET /v1/businesses` route:
 * a provider publishes them so it can be reached. Every channel is optional; a provider chooses
 * which it exposes, and a provider with no location is reached through these alone.
 */
export type BusinessContact = Partial<ContactChannelRecord<string>>;
/** GET /v1/businesses item (non-personal fields only; never the HMAC secret). */
export interface CentralBusiness {
  businessId: string;
  type: string;
  name: string;
  /**
   * `null` when this provider published no location, and that is an ordinary case rather than a
   * defect.
   *
   * It used to be non-optional, so a provider that left location blank was stored as `0, 0` - a
   * legal coordinate in the Gulf of Guinea - and rendered as a pin there. Never substitute a
   * fallback coordinate for `null`; a location we do not have must not be drawn as one we do.
   */
  geo: BusinessGeo | null;
  /** Optional on the wire: a server predating this field sends no `contact` key at all. */
  contact?: BusinessContact;
  services: string[];
  apiBaseUrl: string;
  domain: string;
  documentStores: string[];
  hmacKeyId: string;
}
export interface BusinessesResp {
  businesses: CentralBusiness[];
}
/**
 * GET /v1/businesses query filters.
 *
 * There is deliberately NO `near`/`radius` here. The route still accepts both and still filters
 * server-side, but it is public and unauthenticated, so sending a position there discloses where the
 * caller is standing to anyone in the request path. Distance is computed on the device instead -
 * `packages/ui/src/geo/` - over a provider set whose request is identical whoever makes it.
 *
 * Re-adding these fields is the one change that reopens the leak, because `qs()` in `central.ts`
 * would then have somewhere to read them from. See the note there and the deprecation note on
 * `BusinessesQuery` in `stacks/admin/api/src/routes.rs`.
 */
export interface BusinessesQuery {
  type?: string;
}
export interface RegisterBusinessReq {
  type: string;
  name: string;
  /**
   * Optional, and BOTH-OR-NEITHER: omit `lat` and `lng` together to register a provider with no
   * location. The server rejects a half-set pair with 400 - one coordinate is not a place.
   */
  lat?: number;
  lng?: number;
  contact?: BusinessContact;
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
/** One row whose stored location an operator has to answer for. */
export interface BusinessLocationReviewRow {
  businessId: string;
  type: string;
  name: string;
  domain: string;
  geo: BusinessGeo | null;
  /** Whether it published any contact channel - i.e. whether "no location" would leave it reachable. */
  hasContact: boolean;
}
/**
 * GET /v1/admin/businesses/location-review — rows stored at exactly `0, 0`.
 *
 * Not a defect list and not a repair: `0, 0` is a legal coordinate in the Gulf of Guinea AND the
 * value a blank location was stored as before the field became optional. No code can distinguish
 * them, so this route asks rather than decides. Each row needs an operator answer of "this pin is
 * correct", "this pin is wrong, here is the right one", or "this provider has no location".
 */
export interface BusinessLocationReviewResp {
  totalBusinesses: number;
  needsReview: number;
  businesses: BusinessLocationReviewRow[];
  reason: string;
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

  // ---- the DNS legitimacy trace (advisory gate) ----
  //
  // The DNS check never blocks onboarding: an organisation is routinely KYC-approved before its DNS
  // team publishes anything, and a hard block just drives operators to a bypass. What keeps that from
  // being fail-open is this trace.
  /**
   * LATEST observed DNS state: "verified" | "notListed" | "couldNotCheck" (empty == never checked).
   * Safe for the future daily re-check job to overwrite, so a binding can turn verified with no admin
   * action.
   */
  dnsState?: string;
  /** Unix seconds of the observation in `dnsState` (0 == never checked). */
  dnsCheckedAt?: number;
  /**
   * IMMUTABLE: the state observed at the instant whitelisting happened. Never overwritten by the
   * re-check job, so "whitelisted while DNS was unverified" stays legible even after the binding later
   * turns verified.
   */
  dnsStateAtApproval?: string;
  /** IMMUTABLE: the admin explicitly confirmed and proceeded despite a non-verified observation. */
  dnsProceededUnverified?: boolean;
}
export interface IssuerApplicationsResp {
  applications: IssuerApplicationListItem[];
}
/**
 * The 409 the approve route returns when the live DNS observation is not `verified` and the admin has
 * not yet confirmed. NOT a refusal — a request for a deliberate act, carrying exactly what was OBSERVED
 * so the prompt can state the observation rather than a verdict about the organisation.
 */
export interface DnsConfirmationRequired {
  error: "dnsConfirmationRequired";
  /** "notListed" | "couldNotCheck" — which of the two non-verified outcomes actually occurred. */
  dnsState: string;
  domain: string;
  documentStore: string;
  /** The exact TXT value the domain must publish, for actionable operator copy. */
  expectedTxt: string;
  retryWith: { proceedWithoutDns: boolean };
}

/** POST /v1/issuer-applications/{id}/approve → on-chain whitelistFor per (address,recordType). */
export interface ApproveApplicationResp {
  status: "approved";
  whitelistTxs: string[];
  /** The REAL DNS observation at approval time: "verified" | "notListed" | "couldNotCheck". */
  dnsState?: string;
  dnsCheckedAt?: number;
  /** True when this issuer was whitelisted on an explicit override rather than a clean pass. */
  dnsProceededUnverified?: boolean;
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
 * What a dispatched registrar action actually did.
 *
 * `disposition:"proposed"` is BOTH the legitimate out-of-band-signing flow and what a stack booted on
 * a key that lost its authority produces, so the backend separates them rather than reporting one
 * signal for two very different situations:
 *   - `executed`               at least one action was broadcast; on-chain state changed.
 *   - `proposed_by_design`     nothing broadcast, and the deployment DECLARES propose-only
 *                              (`ADMIN_PROPOSE_ONLY` / `ALLOW_UNAUTHORIZED_ADMIN_SIGNER`) - a correct
 *                              outcome; hand the calldata to the holder.
 *   - `proposed_unauthorized`  nothing broadcast and propose-only was NOT declared - the hosted signer
 *                              was expected to hold the authority and does not.
 *
 * Only the backend decides which it is; never infer it client-side. Optional so an older backend that
 * sends only `executed` still parses.
 */
export type DispatchOutcome = "executed" | "proposed_by_design" | "proposed_unauthorized";

// ---- the generation-2 ProviderRegistry registrar surface (registry plan C-2) ----

/**
 * `ProviderRegistry.Standing`. Shared by providers and services.
 *
 * `registerProvider` writes `pending`, and only `active` satisfies `canWriteProvider` - so a freshly
 * registered provider can do nothing until the registrar raises its standing. `retired` is terminal.
 * Only `active`/`suspended`/`retired` are settable; the contract refuses the other two.
 */
export type ProviderStanding = "none" | "pending" | "active" | "suspended" | "retired";

/** A provider record as the registrar screen reads it. */
export interface RegistryProvider {
  providerId: string;
  controller: string;
  pendingController: string;
  controllerEpoch: number;
  standing: ProviderStanding;
  /**
   * `provider()` does not revert for an unknown id - it answers a zero-filled struct - so this
   * (`controller != 0`) is the existence answer. Never read `standing` for it.
   */
  registered: boolean;
}

/** The registrar-written identity anchor. NOT `ProviderDirectory`'s provider-written ProfileAnchor. */
export interface ProviderIdentityAnchor {
  digest: string;
  schema: number;
  codec: number;
  hashAlgorithm: number;
  revision: number;
  updatedAtBlock: number;
}

/** One `(recordType, allowed)` pair, as last written by the registrar. */
export interface ServiceCreationApproval {
  recordTypeKey: string;
  /** The human label when the key round-trips to one this deployment knows, else null. */
  recordType: string | null;
  allowed: boolean;
}

/**
 * The service-creation approvals for one provider - deliberately tri-state at the type level.
 *
 * `_serviceCreationApprovals` is private with no getter, so the only direct evidence is the
 * `ServiceCreationApprovalSet` log. A read that FAILED is `unavailable` and carries NO `entries`
 * field, so it cannot be spread into a list as `[]`: "we could not ask" and "nothing is approved"
 * are different facts with different remedies, and only the second is a statement about the provider.
 */
export type ProviderApprovalsRead =
  | { state: "resolved"; entries: ServiceCreationApproval[] }
  | { state: "unavailable"; reason: string };

export interface ProviderRegistrarView {
  provider: RegistryProvider;
  /** Null for an id that is not registered - no anchor is invented for one that has none. */
  identityAnchor: ProviderIdentityAnchor | { unavailable: string } | null;
  approvals: ProviderApprovalsRead;
}

/** GET /v1/admin/providers */
export interface ProvidersResp {
  registry: string;
  providers: ProviderRegistrarView[];
  /**
   * Who holds the registry's `Ownable2Step` owner, read LIVE. Stated up front so the screen can say
   * whether a write will execute or come back as an unsigned proposal BEFORE a form is filled in.
   * `heldByHosted` is null when either side could not be established - never guess.
   */
  authority: {
    target: string;
    owner: string | null;
    hostedSigner: string | null;
    heldByHosted: boolean | null;
    capability: string;
  };
  identitySchema: { schema: number; schemaId: string; hashAlgorithm: number };
}

/** POST /v1/admin/providers */
export interface RegisterProviderReq {
  providerId: string;
  controller: string;
  /** keccak256 of the registrar's canonical identity statement. The text itself is never sent. */
  identityDigest: string;
}
export interface RegisterProviderResp {
  providerId: string;
  controller: string;
  identityDigest: string;
  identitySchema: number;
  identitySchemaId: string;
  /** Always `pending` - registration alone does not let the provider act. */
  standingAfterRegistration: ProviderStanding;
  nextStep: string;
  actions: GovernanceDisposition[];
  outcome?: DispatchOutcome;
  executed?: boolean;
  warning?: string | null;
}

/** POST /v1/admin/providers/:providerId/standing */
export interface ProviderStandingReq {
  standing: Exclude<ProviderStanding, "none" | "pending">;
}
export interface ProviderStandingResp {
  providerId: string;
  standing: ProviderStanding;
  actions: GovernanceDisposition[];
  outcome?: DispatchOutcome;
  executed?: boolean;
  warning?: string | null;
}

/** POST /v1/admin/providers/:providerId/service-approval */
export interface ServiceApprovalReq {
  /** A label ("VACCINATION") or an explicit `0x`+64-hex key. */
  recordType: string;
  /** The intended state, sent explicitly rather than as a toggle. */
  allowed: boolean;
}
export interface ServiceApprovalResp {
  providerId: string;
  recordType: string;
  recordTypeKey: string;
  allowed: boolean;
  actions: GovernanceDisposition[];
  outcome?: DispatchOutcome;
  executed?: boolean;
  warning?: string | null;
}

// ---- services, capabilities and typed resolvers: the rest of the provider journey ----

/**
 * An attached service - a provider-deployed contract bound to its provider record.
 *
 * `attachService` resolves `recordTypeKey`, `factoryGeneration` and `confirmedOwner` off the clone
 * and the pinned factory rather than accepting them from the caller, so every field except the
 * standing is a fact about the contract rather than a claim about it.
 */
export interface ServiceRecord {
  serviceAddress: string;
  providerId: string;
  factoryGeneration: string;
  recordTypeKey: string;
  /** `null` when the key is not one this deployment can name - keccak is one-way, never guess. */
  recordType: string | null;
  confirmedOwner: string;
  domainResolver: string;
  ownerEpoch: number;
  standing: ProviderStanding;
  /** `providerId != 0`. `service()` answers a zero-filled struct for an unknown address. */
  attached: boolean;
}

/**
 * The five lifecycle terms `canIssue` folds, reported APART.
 *
 * Never render them as one "working" bool. Each has a different remedy - a suspended provider is the
 * registrar's to lift, an unconfirmed owner needs `confirmServiceOwner`, an inactive generation is
 * terminal - so a single bool would tell an admin that something is wrong while withholding the only
 * thing that says what to do about it.
 *
 * `hasActiveIssuer` is the one term that is NOT independent of its neighbours: the contract answers
 * `_activeIssuerCount != 0 && _serviceIssuanceEligible(..)`, which re-folds the owner and the
 * standings AND adds the provider's current pointer. So it can be false with all four of the others
 * true, and the remedy in that state is the PROVIDER's `repointService`, not a registrar action.
 */
export interface ServiceEffective {
  providerStanding: ProviderStanding;
  serviceStanding: ProviderStanding;
  factoryActive: boolean;
  ownerConfirmed: boolean;
  hasActiveIssuer: boolean;
}

/** One `(holder, allowed)` capability pair as last written by the registrar. */
export interface CapabilityEntry {
  holder: string;
  allowed: boolean;
}
/**
 * Three states, and the third is not a neighbour of the other two: an empty `resolved` says nobody
 * holds this capability, `unavailable` says we could not ask. Rendering the second as the first
 * states a fact about a service on the strength of a read that never happened.
 */
export type CapabilitiesRead =
  | { state: "resolved"; entries: CapabilityEntry[] }
  | { state: "unavailable"; reason: string };

/** Whether the provider has published this service as its current pointer for its record type. */
export type CurrentPointerRead =
  | { state: "resolved"; service: string; isCurrent: boolean }
  | { state: "unavailable"; reason: string };

export interface ProviderServiceView {
  service: ServiceRecord;
  effective: ServiceEffective | { unavailable: string };
  currentPointer: CurrentPointerRead;
  issuance: CapabilitiesRead;
}
/** GET /v1/admin/providers/:providerId/services */
export interface ProviderServicesResp {
  registry: string;
  providerId: string;
  services: ProviderServiceView[];
}

/** POST /v1/admin/providers/:providerId/services/preflight */
export interface AttachPreflightReq {
  serviceAddress: string;
}
/**
 * What the chain says about a candidate contract, before anything is signed.
 *
 * `verdict` is THREE-valued on purpose. `refused` means the chain would reject this; `couldNotRun`
 * means a read that `attachService` itself makes could not be completed, and the send is still
 * offered - a preflight that refused what the contract would accept is a worse defect than none.
 */
export interface AttachPreflightResp {
  registry: string;
  providerId: string;
  serviceAddress: string;
  /** Non-null only when this address is ALREADY bound to a provider. */
  alreadyAttached: ServiceRecord | null;
  generation:
    | { state: "resolved"; generationId: string; factory: string }
    | { state: "none"; reason: string }
    | { state: "unavailable"; reason: string };
  metadata:
    | { state: "resolved"; owner: string; recordTypeKey: string; recordType: string | null }
    | { state: "refused"; reason: string }
    | { state: "unavailable"; reason: string };
  verdict: "ready" | "refused" | "couldNotRun";
  reason: string;
}

/** POST /v1/admin/providers/:providerId/services */
export interface AttachServiceReq {
  serviceAddress: string;
  /** From the preflight - never typed by hand. */
  generationId: string;
  /** The owner as REVIEWED: a transaction guard, never a selector. */
  expectedOwner: string;
}
export interface AttachServiceResp {
  providerId: string;
  serviceAddress: string;
  generationId: string;
  expectedOwner: string;
  /** Always `pending` - attaching alone lets the service issue nothing. */
  standingAfterAttach: ProviderStanding;
  nextStep: string;
  actions: GovernanceDisposition[];
  outcome?: DispatchOutcome;
  executed?: boolean;
  warning?: string | null;
}

/** POST /v1/admin/services/:serviceAddress/standing */
export interface ServiceStandingReq {
  standing: Exclude<ProviderStanding, "none" | "pending">;
}
export interface ServiceStandingResp {
  serviceAddress: string;
  standing: ProviderStanding;
  actions: GovernanceDisposition[];
  outcome?: DispatchOutcome;
  executed?: boolean;
  warning?: string | null;
}

/** POST /v1/admin/rights/:account/issue - the grant is on the ADDRESS and names no service. */
export interface IssuanceCapabilityReq {
  allowed: boolean;
}
export interface IssuanceCapabilityResp {
  account: string;
  /** Retained alongside `account` and carrying the same value, so existing readers keep working. */
  signer: string;
  allowed: boolean;
  actions: GovernanceDisposition[];
  outcome?: DispatchOutcome;
  executed?: boolean;
  warning?: string | null;
}

/**
 * GET /v1/admin/verifier-capabilities - who may verify, per purpose.
 *
 * Keyed by PURPOSE and never by service: the verify axis is ORTHOGONAL to issuance, so rendering it
 * inside a service row would present it as a property of that service. An issuer is not implicitly a
 * verifier.
 */
export interface VerifierPurposeView {
  purpose: string;
  /** The RAW bytes32 the contract takes; it derives `verificationKey(purpose)` itself. */
  purposeKey: string;
  relayers: CapabilitiesRead;
}
export interface VerifierCapabilitiesResp {
  registry: string;
  purposes: VerifierPurposeView[];
}
/** POST /v1/admin/verifier-capabilities */
export interface VerifierCapabilityReq {
  purpose: string;
  relayer: string;
  allowed: boolean;
}
export interface VerifierCapabilityResp {
  purpose: string;
  purposeKey: string;
  relayer: string;
  allowed: boolean;
  actions: GovernanceDisposition[];
  outcome?: DispatchOutcome;
  executed?: boolean;
  warning?: string | null;
}

/**
 * GET /v1/admin/sbt/mint-role - who may `mintCustodial` dog tags.
 *
 * `mintCustodial` is `onlyRole(ISSUER_ROLE)` on `DogTagSBTConsent` and a fresh deployment grants
 * that role to NOBODY, so an EMPTY resolved holder list is the exact misprovisioning that made a
 * live vet issuance die as a silent estimation revert (2026-08-07). The holders arm is tri-state:
 * a failed enumeration is `unavailable` with its reason, never an empty list.
 */
export interface MintRoleResp {
  sbt: string;
  /** keccak256("ISSUER") — shown so an operator can verify against the contract. */
  roleKey: string;
  holders:
    | { state: "resolved"; accounts: string[] }
    | { state: "unavailable"; reason: string };
  /** The SBT's DEFAULT_ADMIN — the only key that can grant. `null` when unreadable. */
  defaultAdmin?: string | null;
}
/** POST /v1/admin/sbt/mint-role */
export interface MintRoleSetReq {
  /** The vet backend's signing key. */
  signer: string;
  allowed: boolean;
}
export interface MintRoleSetResp {
  sbt: string;
  signer: string;
  allowed: boolean;
  actions: GovernanceDisposition[];
  outcome?: DispatchOutcome;
  executed?: boolean;
  warning?: string | null;
}

export type ResolverKind = "directory" | "domain";
/**
 * GET /v1/admin/resolvers - the typed resolver allowlist.
 *
 * A typed resolver answers nothing until BOTH the registrar approves it here AND each provider or
 * service selects it. The core never clears a stored selection when a resolver is deapproved -
 * which is exactly why approval is a separate fleet-wide lever - so the two halves are reported
 * apart and must never be pre-ANDed into one "working" bool.
 */
export interface ResolverKindView {
  kind: ResolverKind;
  listing:
    | { state: "resolved"; resolvers: { resolver: string; approved: boolean }[] }
    | { state: "unavailable"; reason: string };
}
export interface ResolversResp {
  registry: string;
  kinds: ResolverKindView[];
}
/** POST /v1/admin/resolvers */
export interface ResolverApprovalReq {
  kind: ResolverKind;
  resolver: string;
  approved: boolean;
}
export interface ResolverApprovalResp {
  kind: ResolverKind;
  resolver: string;
  approved: boolean;
  /** Approval is only HALF - the provider must still select it, and the registrar cannot for them. */
  nextStep: string;
  actions: GovernanceDisposition[];
  outcome?: DispatchOutcome;
  executed?: boolean;
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
  /** Immutable generation id (the lowercase factory address); join to status.watchedGenerations. */
  generation: string;
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
  /**
   * The indexer's ready-to-click explorer link for the anchoring tx. Composed UNCONDITIONALLY from
   * `EXPLORER_BASE` (`stacks/indexer/api/src/routes.rs`), so its presence is NOT evidence that the
   * transaction exists - a scripted feed's `0x0800` arrives carrying one too. Never link it directly;
   * go through `txExplorerHref`, which withholds it unless `txHash` is a well-formed 32-byte value.
   */
  txUrl?: string | null;
  /**
   * The owner-blind `Verified` payload. The admin backend proxies the indexer's `/v1/events` body
   * verbatim (`admin_activity` in `stacks/admin/api/src/routes.rs` only inserts directory names), so
   * these arrive on the wire already - they were simply never declared, which left the audit table's
   * Details column empty for the one event type whose identifiers are all it has.
   * See `IndexedEvent` in `stacks/indexer/api/src/events.rs`.
   */
  /** keccak/BN254 purpose key (`verified`); hashed, so prefer a joined human label where one exists. */
  purpose?: string | null;
  /** the one-time verification nullifier (`verified`) - a Poseidon image, unlinkable. */
  nullifier?: string | null;
  /** the proof-bound consent deadline in unix SECONDS (`verified`). */
  deadline?: number | null;
  /** the event's own on-chain `ts` (`rootIssued`/`rootRevoked`/`verified`), == the emit block time. */
  onchainTs?: number | null;
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
/** One exact contract-generation triple admitted by the indexer's role-specific anti-spoof gate. */
export interface IndexerWatchedGeneration {
  /** Immutable generation id; equal to the normalized factory address. */
  generation: string;
  factory: string;
  issuerRegistry: string;
  verificationRegistry: string;
  /** Pre-existing clones admitted for this generation before IssuerCreated discovery catches up. */
  seedClones: string[];
}
/** GET /v1/admin/activity/status - indexer progress + finality watermark (chain-health card). */
export interface IndexerStatus {
  /** the indexed chain's id, or null when the indexer is serving a simulated source. */
  chainId: number | null;
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
  /** Exact generation set being watched; omissions stay visible even when the event feed is empty. */
  watchedGenerations: IndexerWatchedGeneration[];
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
/**
 * The factory-anchored issuer-whitelist pillar's outcome. It is NOT a `FragmentState`, because a
 * caller must be able to tell "we never asked" apart from "we asked and it passed" — a pillar that
 * never ran must never render as one that succeeded.
 *
 * `unavailableNoFactoryConfigured` is the ONLY non-`passed` state that does not fail the credential:
 * it means this verifier has no `FACTORY_ADDR`, which is our own gap and not evidence about the
 * document. `unresolved` (we asked, and could not establish an answer) DOES fail it.
 */
export type IssuerWhitelistState =
  | "passed"
  | "failed"
  | "unresolved"
  | "unavailableNoFactoryConfigured";

/** How the issuing clone was arrived at. Anything but `resolved` means `issuerAddr` is a claim. */
export type IssuerResolution = "resolved" | "noRecord" | "noFactoryConfigured" | "readFailed";

/**
 * Whether the document's own `issuer.documentStore` names the clone the factory resolved. Reported
 * BESIDE the whitelist pillar rather than folded into it, because "the document named a different
 * contract than the chain did" and "the signer is not authorised for this record type" are different
 * accusations with different remedies. `notEvaluated` means no clone was resolved, so there was
 * nothing authoritative to disagree with - not a failure.
 */
export type IssuerStoreAgreement = "matched" | "differs" | "notEvaluated";

export interface ImportVerdict {
  valid: boolean;
  integrity: FragmentState;
  issuance: FragmentState;
  identity: FragmentState;
  ownership: FragmentState;
  /**
   * OPTIONAL for the same reason {@link VerifyCredentialResp.fragments.issuerWhitelistState} is: the
   * backend is separately deployed, so a portal build newer than its API must degrade to an honest
   * unknown rather than throw on a missing key.
   */
  issuerWhitelistState?: IssuerWhitelistState;
  issuerResolution?: IssuerResolution;
  issuerStoreAgreement?: IssuerStoreAgreement;
  /** The clone the on-chain reads were made against — the FACTORY's answer when one was available. */
  issuerAddr: string;
}

// --------------------------------------------------------------------------------------------
// Shop CRM — clients / appointments / verification history (`stacks/vet/api/src/crm.rs`).
// --------------------------------------------------------------------------------------------

/** One page of a CRM list query. `total` is the FULL match count, for the pager. */
export interface CrmPage<T> {
  rows: T[];
  total: number;
  limit: number;
  offset: number;
}

export interface ClientPet {
  petId: string;
  name: string;
  species: string;
  breed: string;
  sex: string;
  dateOfBirth: string;
  notes: string;
  /** the pet's DogTag id, if the owner holds one */
  dogTagId: string | null;
  /**
   * The animal's microchip code as the shop recorded it, or `null`.
   *
   * `null` is NORMAL, not a gap to chase — many animals are not chipped. What it costs is the
   * cross-check against the credential's own microchip leaf
   * ({@link MicrochipCheck}), never the ability to link a tag.
   */
  microchipCode: string | null;
}

/**
 * Whether the credential attached to a tag describes the animal on the shop's own record.
 *
 * FOUR STATES, and `notComparable` must never render as either of its neighbours. A check that did
 * not run is not a check that passed — and, on this surface especially, it is not a failure either:
 * refusing every unchipped cat would make the field unusable.
 *
 * `unrecognisedCredentialLeaf` is separate from `notComparable` on purpose, and the separation is
 * load-bearing rather than tidy. This check shipped once reading a single key path no real issuer
 * emits, so it was inert on every real credential — and what hid that is that "the credential has no
 * microchip" (ordinary, quiet, benign) and "our reader could not find the microchip it is carrying"
 * produced the same answer. So a reader that cannot read gets its OWN state, names the paths it
 * found, and is never renderable as "nothing to compare".
 *
 * Emitted only by the routes that WRITE the tag↔pet binding (`linkPetDogTag`, `POST /import/pull`)
 * and inside their refusal bodies. Deliberately absent from {@link CrmPet} list rows, so an absent
 * key can never be mistaken for a check that passed.
 */
export type MicrochipCheck =
  | {
      state: "matched";
      /** The code both sides carry. */
      microchip: string;
      detail: string;
    }
  | {
      state: "mismatch";
      petMicrochip: string;
      credentialMicrochip: string;
      detail: string;
    }
  | {
      state: "notComparable";
      reason: MicrochipNotComparable;
      /**
       * Whether this is a failure to LOOK (`true`) or an ordinary fact about what exists (`false`).
       *
       * Drives the tone: a fact is neutral, a failure gets the warning treatment. Read it from here
       * rather than re-deriving it from `reason`, so a reason added later cannot silently default to
       * the wrong one.
       */
      isFailure: boolean;
      /** The sentence to show. Always says the two could not be compared, and why. */
      detail: string;
    }
  | {
      state: "unrecognisedCredentialLeaf";
      /**
       * The microchip-shaped key path(s) the credential carries that this system cannot read.
       *
       * Named so the remedy is obvious from the message alone: the path belongs in the backend's
       * recognised set (`RECOGNISED_MICROCHIP_SUFFIXES` in `stacks/vet/api/src/microchip.rs`).
       */
      keyPaths: string[];
      detail: string;
      /**
       * Deliberately NO `reason`/`isFailure` pair. Those belong to `notComparable`, and giving this
       * state their shape is precisely how a client's fallback branch would absorb it back into
       * "nothing to compare" — the camouflage it exists to escape.
       */
      isFailure?: undefined;
    };

/**
 * Why the two sides could not be compared.
 *
 * The first four are FACTS about what exists (`isFailure: false`); the last three are failures to
 * look (`isFailure: true`). They do not collapse into each other: `noCredentialHeld` says the shop
 * holds nothing, while `cannotLookUpByFieldElement` says it cannot ask — the held-document cache is
 * keyed by the tag handle and handle -> field element is a hash that cannot be inverted.
 */
export type MicrochipNotComparable =
  | "noCredentialHeld"
  | "credentialHasNoMicrochip"
  | "petHasNoMicrochip"
  | "noLinkedPet"
  | "cannotLookUpByFieldElement"
  | "couldNotRead"
  | "credentialUnreadable";

/**
 * One row of the PETS collection (`GET /pets`, `GET /pets/{petId}`).
 *
 * A pet is stored embedded in its owner's document but is addressed in its own right, so every row
 * carries the owner denormalized — the same denormalization {@link CrmAppointment.clientName} uses.
 * That is what makes the pet -> owner half of the round trip a link rather than a second fetch.
 */
export interface CrmPet extends ClientPet {
  clientId: string;
  clientName: string;
  /** the OWNING CLIENT's `updatedAt`: a pet has no timestamp of its own. */
  updatedAt: number;
}

/** Pet fields on create. A pet must belong to someone, so `clientId` is required. */
export interface PetCreateInput extends ClientPetInput {
  clientId: string;
}

/**
 * Pet fields on edit. Every field is optional and an ABSENT field is left alone — unlike
 * `PUT /clients/{id}`, which replaces the whole document (and so deletes any pet left out of it).
 */
export interface PetPatchInput {
  name?: string;
  species?: string;
  breed?: string;
  sex?: string;
  dateOfBirth?: string;
  notes?: string;
  /** An explicit `""` CLEARS the code — the only way out of a mismatch typed onto the wrong pet. */
  microchipCode?: string;
}

export interface PetListQuery extends ClientListQuery {
  clientId?: string;
}

/**
 * The credentials this shop HOLDS for a pet's DogTag (`GET /pets/{petId}/credentials`) — the wrapped
 * documents `POST /import/pull` accepted and stored.
 *
 * Deliberately no verdict field. A credential's validity is an on-chain fact that can change after the
 * import (a root may be revoked the next day), so a stored verdict would be a stale claim; the caller
 * re-reads the chain for each document instead.
 */
export interface PetCredentialsResp {
  /** The tag the lookup used, or `null` when the pet has none — distinguishing "no tag to look up" from "no credentials". */
  dogTagId: string | null;
  /** Wrapped credential documents, as stored. Parsed by the caller (`@dogtag/standard`). */
  credentials: Record<string, unknown>[];
}

export interface CrmClient {
  clientId: string;
  name: string;
  email: string;
  phone: string;
  address: string;
  notes: string;
  pets: ClientPet[];
  createdAt: number;
  updatedAt: number;
}

/** Pet fields on write. Echo `petId` on an edit to keep the pet's identity (and its links). */
export interface ClientPetInput {
  petId?: string;
  name: string;
  species?: string;
  breed?: string;
  sex?: string;
  dateOfBirth?: string;
  notes?: string;
  dogTagId?: string | null;
  microchipCode?: string | null;
}

export interface ClientInput {
  name: string;
  email?: string;
  phone?: string;
  address?: string;
  notes?: string;
  pets?: ClientPetInput[];
}

export const APPOINTMENT_STATES = [
  "scheduled",
  "confirmed",
  "in_progress",
  "completed",
  "cancelled",
  "no_show",
] as const;
export type AppointmentStatus = (typeof APPOINTMENT_STATES)[number];

export interface CrmAppointment {
  appointmentId: string;
  /**
   * The shop client this booking belongs to, or `""` for an UNASSIGNED one.
   *
   * Empty only ever comes from an `.ics` import: a calendar invite names an event, not a DogTag
   * client, and the import refuses to fabricate a directory entry to fill the column. Render it via
   * `AppointmentClient` rather than linking to `/clients/${clientId}` — an empty id produces a dead
   * link with no label.
   */
  clientId: string;
  /** denormalized so a list renders without a per-row client fetch */
  clientName: string;
  petId: string | null;
  petName: string;
  service: string;
  /** UNIX SECONDS (the calendar range-queries this) */
  startAt: number;
  endAt: number;
  status: AppointmentStatus;
  notes: string;
  groomer: string;
  createdAt: number;
  updatedAt: number;
  /** `"ics"` for a booking created by a calendar import; absent/null when booked in the portal. */
  source?: string | null;
  /** The originating calendar's `UID` for an imported booking — what makes a re-import idempotent. */
  externalUid?: string | null;
  /** present on the single-appointment read only */
  verifications?: CrmVerification[];
}

export interface AppointmentInput {
  clientId: string;
  petId?: string | null;
  service?: string;
  startAt: number;
  /** omit for a default one-hour slot */
  endAt?: number;
  status?: AppointmentStatus;
  notes?: string;
  groomer?: string;
}

/**
 * A verification the shop performed. Holds the public on-chain facts plus the keyPaths the owner
 * chose to disclose — never their values, and never the owner's identity or wallet, which the
 * protocol withholds from a verifier.
 */
export interface CrmVerification {
  verificationId: string;
  appointmentId: string | null;
  clientId: string | null;
  clientName: string;
  petId: string | null;
  petName: string;
  purpose: string;
  recordType: string;
  status: "pending" | "recording" | "recorded" | "error";
  /** on `error` this carries the failure message, mirroring the verify session */
  txHash: string | null;
  nullifier: string | null;
  dogTagId: string | null;
  /** EMPTY on an ordinary owner-hidden verification — that emptiness IS the privacy guarantee. */
  disclosedKeyPaths: string[];
  createdAt: number;
  updatedAt: number;
}

/** Filters accepted by the list queries. Absent/blank fields are not applied. */
export interface ClientListQuery {
  q?: string;
  limit?: number;
  offset?: number;
}
export interface AppointmentListQuery extends ClientListQuery {
  clientId?: string;
  petId?: string;
  status?: AppointmentStatus;
  /** inclusive lower / exclusive upper bound on startAt (unix seconds) */
  from?: number;
  to?: number;
}
export interface VerificationListQuery extends ClientListQuery {
  clientId?: string;
  /**
   * Restrict to ONE pet's verifications. A client may bring several pets, each holding its own
   * DogTag, so this is a genuinely narrower question than `clientId` — and the pet detail page must
   * ask the narrow one or it would present another pet's checks as this pet's.
   */
  petId?: string;
  appointmentId?: string;
  status?: CrmVerification["status"];
  purpose?: string;
  from?: number;
  to?: number;
}

// --------------------------------------------------------------------------------------------
// `.ics` calendar interop (`stacks/vet/api/src/calendar_ics.rs`).
// --------------------------------------------------------------------------------------------

/**
 * The published `.ics` subscription feed's state.
 *
 * `token` is a CREDENTIAL: anyone holding the URL it builds can read the shop's whole schedule. It
 * is returned to the operator's own session so the portal can display and copy the link, and is
 * revoked by `revokeIcsFeed` / replaced by `rotateIcsFeed`.
 */
export interface IcsFeedResp {
  enabled: boolean;
  token: string | null;
  /**
   * API-relative path of the feed, e.g. `/calendar/feed/<token>.ics`. The portal composes the
   * absolute URL against its own origin + API base, because that is what a subscriber actually
   * reaches — the backend's configured `DEPLOYMENT_URL` can legitimately differ (dev proxy, tunnel).
   */
  path: string | null;
}

/**
 * A minted per-appointment CLIENT handoff (`stacks/vet/api/src/appointment_share.rs`).
 *
 * `qrUrl` is present IF AND ONLY IF this deployment has a base a client's phone could actually
 * reach. It is `null` when `DEPLOYMENT_URL` is unset or is a loopback address, and the portal must
 * key its QR off that field alone — rendering a QR from `url`, or from the portal's own origin,
 * reintroduces the defect this shape exists to prevent: a scannable code that encodes a host the
 * scanning phone cannot resolve, which still looks like a working link.
 *
 * `url` is the absolute link where one exists at all. On a loopback deployment it is populated while
 * `qrUrl` is not: the link genuinely works on the machine serving it, which is where a dev run needs
 * it, and `qrUnavailableReason` says so.
 */
export interface AppointmentShareResp {
  appointmentId: string;
  /** The handoff secret (32 hex). Anyone holding it can read THIS booking — and only this one. */
  token: string;
  /** Backend-relative path of the page, e.g. `/a/<token>`. */
  path: string;
  /** Backend-relative path of the calendar file, e.g. `/a/<token>.ics`. */
  icsPath: string;
  /** Absolute URL to encode in a QR, or `null` when no reachable base is configured. */
  qrUrl: string | null;
  /** Absolute URL to display/copy, or `null` when there is no base at all. */
  url: string | null;
  /** Why no QR was drawn, in words naming the fix. `null` exactly when `qrUrl` is present. */
  qrUnavailableReason: string | null;
  /** Unix seconds after which the link stops resolving. */
  expiresAt: number;
}

/** One already-normalized event from a parsed `.ics` (see `packages/ui/src/calendar/ics.ts`). */
export interface IcsImportEventInput {
  uid: string;
  summary?: string;
  description?: string;
  location?: string;
  /** unix seconds, resolved from the source event's own TZID/DATE semantics */
  startAt: number;
  endAt?: number;
  allDay?: boolean;
  /** the source carried an RRULE; the import does NOT expand it */
  recurring?: boolean;
  status?: string;
}

/** What an import did (or, for a dry run, WOULD do). */
export interface IcsImportResp {
  dryRun: boolean;
  total: number;
  created: number;
  updated: number;
  cancelled: number;
  skipped: number;
  /** events imported as a SINGLE occurrence because recurrence is not expanded */
  recurringNotExpanded: number;
  allDay: number;
}
