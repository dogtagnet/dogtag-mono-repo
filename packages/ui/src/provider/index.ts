/**
 * The provider self-service engine (registry-plan S-15).
 *
 * Four flows, against the generation-2 registry set:
 *   1. the provider deploys its OWN clone          - `deployPlan.ts`
 *   2. the provider repoints its recorded contract - `cloneProvenance.ts`
 *   3. the provider claims a domain                - `domainClaim.ts`
 *   4. the provider publishes pin/contacts/profile - `directoryPlan.ts`
 *
 * Both 3 and 4 sit behind a SELECTION the provider makes for itself - which typed resolver holds its
 * domain, and which holds its listing - and that is `resolverSelection.ts`. Until it shipped neither
 * flow could be completed by anybody, because the registrar's approval half existed and the
 * provider's selection half had no surface at all.
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
  ResolverKind,
  type ProviderRecord,
  type ResolverListing,
  type ServiceRecord,
} from "./readers";

export {
  assessResolverSelection,
  canStopUsing,
  describeSelection,
  kindForScope,
  selectionChangeRefusal,
  type ResolverSelectionPlan,
  type ResolverSelectionScope,
  type ResolverSelectionState,
} from "./resolverSelection";

export {
  checkBlock,
  briefActionBlock,
  firstSentence,
  renderReason,
  sequenceReasons,
  describeActionBlock,
  describePlanRetirement,
  planGateState,
  sendBlock,
  type ActionBlock,
  type ChainCheck,
  type PlanGateState,
  type PlanRetirementReason,
  type RenderedReason,
} from "./actionAvailability";

export {
  PROVIDER_ID_STORAGE_PREFIX,
  providerIdStorageKey,
  recallProviderId,
  rememberProviderId,
} from "./providerIdMemory";

export {
  assessCandidateClone,
  ATTACHMENT_IS_A_DOGTAG_STEP,
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
  DEPLOYED_CONTRACT_NEXT_STEP,
  MAX_CLONE_NONCE,
  nextContractNumber,
  parseContractNumber,
  readDeploymentHistory,
  type ContractNumberInput,
  type DeployedContract,
  type DeploymentHistory,
  type IssuerCreationLog,
  type NextContractNumber,
} from "./deploymentHistory";

export {
  assessDomainClaim,
  canWithdraw,
  describeDisposition,
  DOMAIN_REGISTER_NEEDS_TURNING_ON,
  MAX_DOMAIN_LENGTH,
  validateDomain,
  type DomainClaimAssessment,
  type DomainValidation,
} from "./domainClaim";

export {
  CONTACT_ONLY_NOTICE,
  CONTACTS_ARE_ANCHORED_NOT_SERVED,
  COORDINATE_SCALE,
  DIRECTORY_NEEDS_TURNING_ON,
  MAX_SCANNED_LOCATION_NUMBERS,
  mirrorPublicationRefusal,
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
  PROVIDER_PERMISSION_DIRECTORY_RESOLVER,
  SERVICE_PERMISSION_DOMAIN_RESOLVER,
  type LiveReaderOptions,
  type ProviderContracts,
} from "./liveReader";

export {
  createdAddressMeaning,
  hasNoHash,
  isUnsettled,
  mayContinueAfter,
  outcomeFromReceiptStatus,
  sendExplorerHref,
  sendRecord,
  sendStateLabel,
  type SendRecord,
  type SendState,
} from "./sendOutcome";
