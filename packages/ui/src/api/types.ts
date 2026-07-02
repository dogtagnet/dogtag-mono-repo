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
