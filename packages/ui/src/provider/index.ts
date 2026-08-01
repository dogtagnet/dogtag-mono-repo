/**
 * The provider self-service engine (registry-plan S-15).
 *
 * Four flows, against the generation-2 registry set:
 *   1. the provider deploys its OWN clone          - `deployPlan.ts`
 *   2. the provider repoints its recorded contract - `cloneProvenance.ts`
 *   3. the provider claims a domain                - `domainClaim.ts`
 *   4. the provider publishes pin/contacts/profile - `directoryPlan.ts`
 *
 * Everything here is pure and reader-injected, so every decision is testable without a node. The
 * chain seam is `readers.ts`; the live viem binding is `liveReader.ts`.
 */

export {
  foldVerdict,
  providerCheck,
  reasonFrom,
  type CheckOutcome,
  type CloneLifecycle,
  type ProviderCheck,
  type ProviderCheckId,
  type ProviderVerdict,
} from "./types";

export {
  DomainDisposition,
  STANDING_LABEL,
  Standing,
  ZERO_ADDR,
  ZERO_PROVIDER_ID,
  ZERO_WORD,
  type Address,
  type DirectoryPin,
  type DomainClaimStanding,
  type EffectiveService,
  type HexWord,
  type ProfileAnchorRecord,
  type ProviderChainReader,
  type ProviderRecord,
  type ServiceRecord,
} from "./readers";

export {
  assessCandidateClone,
  nextStepFor,
  REPOINT_SCOPE_NOTICE,
  type CloneAssessment,
  type CloneAssessmentRequest,
} from "./cloneProvenance";

export {
  ATTACHMENT_IS_NOT_SELF_SERVICE,
  planCloneDeployment,
  type DeployPlan,
  type DeployPlanRequest,
} from "./deployPlan";

export {
  assessDomainClaim,
  canWithdraw,
  describeDisposition,
  MAX_DOMAIN_LENGTH,
  validateDomain,
  type DomainClaimAssessment,
  type DomainValidation,
} from "./domainClaim";

export {
  CONTACT_ONLY_NOTICE,
  CONTACTS_ARE_ANCHORED_NOT_SERVED,
  COORDINATE_SCALE,
  MAX_SCANNED_LOCATION_NUMBERS,
  planDirectoryPublication,
  readListingState,
  toContractCoordinate,
  WITHDRAW_LOCATION_NOTICE,
  type DigestFn,
  type DirectoryListingState,
  type DirectoryPublicationPlan,
  type DirectoryPublicationRequest,
  type DirectoryStep,
} from "./directoryPlan";

export {
  createLiveProviderReader,
  type LiveReaderOptions,
  type ProviderContracts,
} from "./liveReader";

export {
  mayContinueAfter,
  outcomeFromReceiptStatus,
  sendExplorerHref,
  sendRecord,
  sendStateLabel,
  type SendRecord,
  type SendState,
} from "./sendOutcome";
