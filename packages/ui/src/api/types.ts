/**
 * Wire types mirroring the vet backend (`stacks/vet/api/src/routes.rs`) and the central
 * admin API (`stacks/admin/api/src/routes.rs`) JSON contracts. Field names match the Rust
 * serde renames exactly.
 */

export type SigningMode = "wallet" | "backend";
export type VerifyMode = "normal" | "zk";
export type RecordStatus = "prepared" | "issued" | "revoked";

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
}
export interface ShareResp {
  qrUrl: string;
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
export type AdminLoginResp = LoginResp;

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
}
export interface RejectApplicationResp {
  status: "rejected";
}
export interface DelistApplicationResp {
  status: "delisted";
  delistTxs: string[];
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
