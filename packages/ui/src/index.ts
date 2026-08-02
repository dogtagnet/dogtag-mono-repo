// utilities
export { cn } from "./lib/cn";

// theme
export { ThemeProvider, useTheme, type Theme } from "./theme/ThemeProvider";
export { ThemeToggle } from "./theme/ThemeToggle";

// primitives
export { Button, buttonVariants, type ButtonProps } from "./components/Button";
export { Spinner } from "./components/Spinner";
export {
  Card,
  CardHeader,
  CardTitle,
  CardDescription,
  CardContent,
  CardFooter,
} from "./components/Card";
export { Input, type InputProps } from "./components/Input";
export { Label } from "./components/Label";
export {
  Select,
  SelectGroup,
  SelectValue,
  SelectTrigger,
  SelectContent,
  SelectItem,
} from "./components/Select";
export {
  Dialog,
  DialogTrigger,
  DialogClose,
  DialogPortal,
  DialogOverlay,
  DialogContent,
  DialogHeader,
  DialogFooter,
  DialogTitle,
  DialogDescription,
} from "./components/Dialog";
export { Badge, badgeVariants, type BadgeProps } from "./components/Badge";
export {
  DomainBindingBadge,
  type DomainBindingBadgeProps,
} from "./components/DomainBindingBadge";
export {
  bindingExplanation,
  bindingLine,
  bindingProvenanceLine,
  bindingTone,
  displayIssuerName,
  expectedTxtName,
  isDomainVerified,
  type BindingTone,
  type IssuerDomainBinding,
  type IssuerDomainBindingState,
  type IssuerIdentity,
} from "./domain/issuerDomainBinding";
// The microchip cross-check's PRESENTATION — four states, and "not compared" is never rendered as
// either neighbour. See the module header for why an absent microchip is normal.
export {
  microchipCheckFromError,
  microchipConfirmsAnimal,
  microchipExplanation,
  microchipHeadline,
  microchipTone,
  type MicrochipTone,
} from "./domain/microchipCheck";
export {
  Table,
  TableHeader,
  TableBody,
  TableRow,
  TableHead,
  TableCell,
} from "./components/Table";
export { Tabs, TabsList, TabsTrigger, TabsContent } from "./components/Tabs";
export { ToastProvider, useToast, type ToastItem } from "./components/Toast";
export { QrCode, type QrCodeProps } from "./components/QrCode";
export { AppShell, type AppShellProps, type NavItem } from "./components/AppShell";
export {
  RpcEndpointSettingsCard,
  type RpcEndpointSettingsCardProps,
} from "./components/RpcEndpointSettingsCard";

// wallet
export {
  roax,
  ROAX_CHAIN_ID,
  ROAX_CHAIN_ID_HEX,
  ROAX_ADD_CHAIN_PARAMS,
  roaxAddChainParams,
  explorerTxUrl,
  explorerAddressUrl,
} from "./wallet/chain";
export {
  DEFAULT_ROAX_RPC_URL,
  ROAX_RPC_STORAGE_KEY,
  createGuardedRoaxRpcRequest,
  getRoaxRpcPreference,
  getRoaxRpcPreferenceRevision,
  guardedRoaxTransport,
  normalizeRpcUrl,
  resetRoaxRpcPreference,
  resolveRoaxRpcEndpoint,
  setRoaxRpcPreference,
  subscribeRoaxRpcPreference,
  validateAndSaveRoaxRpcPreference,
  type GuardedRoaxRpcRequestOptions,
  type ResolvedRoaxRpcEndpoint,
  type ResolveRoaxRpcEndpointOptions,
  type RoaxRpcPreference,
  type RpcEndpointFailure,
  type RpcFetch,
  RpcPreferenceOperationSupersededError,
  type StorageLike,
  type ValidateAndSaveRoaxRpcPreferenceOptions,
} from "./chain/rpcEndpoint";
export { useRoaxRpcPreference } from "./chain/useRoaxRpcPreference";
export {
  useRoaxRpcSettings,
  type RoaxRpcSettingsMessage,
} from "./chain/useRoaxRpcSettings";

// on-chain provenance (audit surfaces: government Oversight, vet/groomer Traceability)
export {
  isHash32,
  isEvmAddress,
  chainProvenance,
  txExplorerHref,
  addressExplorerHref,
  addressChainRef,
  txChainRef,
  NOT_AN_ADDRESS_REASON,
  NOT_A_TX_HASH_REASON,
  shortHex,
  shortValue,
  isOpaqueIdentifier,
  formatChainTime,
  formatRelativeTime,
  emittingContractRole,
  emittingCloneName,
  eventDetailFields,
  joinedDetailContext,
  type ChainProvenance,
  type ChainRef,
  type ChainEventLike,
  type ChainDetailField,
  type ChainDetailContext,
  type ChainLocalJoinLike,
} from "./chain/provenance";
export {
  ChainValue,
  ChainTime,
  CopyButton,
  ProvenanceBadge,
  TxRef,
  type ChainValueProps,
  type ChainTimeProps,
  type CopyButtonProps,
  type ProvenanceBadgeProps,
  type TxRefProps,
} from "./chain/ChainValue";

// on-chain DogTag discovery — bounded, cancellable scan of what exists against one tag
export {
  discoverTag,
  chunkRanges,
  isOwnSigner,
  onchainDogTagId,
  resolveDogTagId,
  tagStatusLabel,
  roaxTagDiscoveryReader,
  DISCOVERY_ADDRESSES,
  DEFAULT_LOOKBACK_BLOCKS,
  DEFAULT_CHUNK_BLOCKS,
  TAG_STATUS_LABELS,
  attributeSigner,
  type SignerAttribution,
  type DiscoverTagArgs,
  type DiscoveryAddresses,
  type DiscoveryCoverage,
  type DiscoveryProgress,
  type DiscoveredEvent,
  type DiscoveredMint,
  type DiscoveredStatusChange,
  type DiscoveredBurn,
  type DiscoveredVerification,
  type DiscoveredProfileCredential,
  type ResolvedDogTagId,
  type TagDiscoveryReader,
  type TagDiscoveryResult,
  type TagStatusLabel,
} from "./chain/tagDiscovery";

export { createWalletConfig, roaxNetwork, type WalletConfigOptions } from "./wallet/config";
export {
  DEPLOYED_ADDRESSES,
  recordTypeKey,
  roaxPublicClient,
  isWhitelistedFor,
  isRootValid,
  isRootRevoked,
  issuedAtOf,
  type IsWhitelistedForArgs,
} from "./wallet/contracts";
export {
  verifyCredentialOnchain,
  roaxIssuerChainReader,
  type IssuerChainReader,
  type VerifyCredentialOnchainArgs,
} from "./wallet/verifyCredential";
export { WalletProvider } from "./wallet/WalletProvider";
export { WalletButton, shortAddress } from "./wallet/WalletButton";
export { useRoaxChain, type UseRoaxChainResult } from "./wallet/useRoaxChain";
// The admin verification bench: the real verifier, observed. See `wallet/verificationBench.ts` for why
// it wraps `verifyCredentialOnchain` rather than reimplementing any part of it.
export {
  runVerificationBench,
  recordingReader,
  docFromShareResponse,
  validUntilOf,
  VALID_UNTIL_KEYPATHS,
  todayIso,
  type BenchCheck,
  type ValidityWindow,
  type BenchCheckId,
  type BenchInput,
  type BenchReport,
  type ChainRead,
  type CheckOutcome,
  type EvidenceLine,
  type GrantHistoryReader,
  type IssuerAuthorityReader,
} from "./wallet/verificationBench";
export {
  BENCH_MUTATIONS,
  type BenchMutation,
  type UncaughtBy,
} from "./wallet/benchMutations";
// The adversarial catalogue: fraudulent records with scripted chains, each naming the check that must
// refuse it. Exported so the admin page can RUN it in the browser rather than describing it.
export {
  BENCH_SCENARIOS,
  genuineCredential,
  runBenchScenario,
  type BenchScenario,
  type ScenarioBlindSpot,
  type ScenarioWorld,
} from "./wallet/benchScenarios";

// domain components
export { SigningModeToggle, type SigningModeToggleProps } from "./domain/SigningModeToggle";
export { StatusPanel, formatPlasma, type StatusPanelProps } from "./domain/StatusPanel";
export { VerifyFlow, type VerifyFlowProps, type VerifyPurpose } from "./domain/VerifyFlow";
export {
  VerificationHistoryPanel,
  type VerificationHistoryPanelProps,
} from "./domain/VerificationHistoryPanel";
export {
  CredentialVerifyPanel,
  type CredentialVerifyPanelProps,
} from "./domain/CredentialVerifyPanel";
export {
  IssuanceStatusPanel,
  type IssuanceStatusPanelProps,
} from "./domain/IssuanceStatusPanel";
export {
  CustodyUnlockForm,
  CustodyUnlockDialog,
  CustodyUnlockPanel,
  CustodyLockedBanner,
  type CustodyUnlockFormProps,
  type CustodyUnlockDialogProps,
  type CustodyUnlockPanelProps,
  type CustodyLockedBannerProps,
} from "./domain/CustodyUnlock";

// custody lock / unlock routing
export {
  UNLOCK_PATH,
  NEXT_PARAM,
  buildUnlockPath,
  custodyStateFromSigners,
  isCustodyLockedError,
  isNoSealError,
  isWrongPassphraseError,
  sanitizeNextPath,
  type CustodyState,
} from "./custody/lock";

// api
export { createApiClient, type ApiClient, type ApiClientOptions } from "./api/client";
export {
  createCentralClient,
  type CentralClient,
  type CentralClientOptions,
} from "./api/central";
export * from "./api/types";

// provider directory — full-set reads, explicit empty/unavailable states, and block-anchored TTL cache
export {
  centralDirectory,
  onchainDirectory,
  createMemoryProviderDirectoryCache,
  withProviderDirectoryCache,
  // a location-less provider is listed and contactable, but never placed — see directory/providers.ts
  blankContactFields,
  contactRequestFields,
  locationRequestFields,
  parseLocationInput,
  type LocationInput,
  hasDirectionsDestination,
  isContactable,
  isUnreachableProvider,
  partitionByLocatability,
  providerContactEntries,
  providerPosition,
  PROVIDER_CONTACT_CHANNELS,
  type CentralDirectoryOptions,
  type OnchainDirectoryOptions,
  type ContactChannelRecord,
  type DirectoryObservation,
  type DirectoryProvider,
  type DirectoryProviderContact,
  type ProviderContactChannel,
  type ProviderContactEntry,
  type ProviderDirectory,
  type ProviderDirectoryCache,
  type ProviderDirectoryCacheEntry,
  type ProviderDirectoryCacheOptions,
  type ProviderDirectoryEmpty,
  type ProviderDirectoryFound,
  type ProviderDirectoryResult,
  type ProviderDirectorySnapshot,
  type ProviderDirectorySource,
  type ProviderDirectoryUnavailable,
  type ProviderDirectoryUnavailableReason,
} from "./directory";

// calendar grid arithmetic — DST-safe day/week/month stepping (see calendar/grid.ts)
export {
  DAY_SECS,
  addDays,
  addMonths,
  daysBetween,
  monthGrid,
  startOfDay,
  startOfMonth,
  startOfWeek,
} from "./calendar/grid";

// calendar interop — `.ics` parsing for import (see calendar/ics.ts for why it runs client-side)
export {
  parseIcs,
  parseInstant,
  parseDuration,
  parseContentLine,
  unfoldLines,
  unescapeText,
  zonedToUnix,
  type IcsImportEvent,
  type IcsParseResult,
  type IcsSkippedEvent,
  type ParsedInstant,
} from "./calendar/ics";

// on-device geo core - distance, bearing, formatting, sorting, geohash. Pure, no I/O.
// The user's position is computed with, never transmitted: see `geo/index.ts` for the boundary and
// the deprecated server-side `near=` filter it replaces.
export {
  EARTH_RADIUS_KM,
  MAX_DISTANCE_KM,
  GEOHASH_BASE32,
  MAX_GEOHASH_PRECISION,
  COMPASS_POINTS_8,
  COMPASS_POINTS_16,
  FEET_PER_KM,
  KM_PER_MILE,
  haversineKm,
  isValidLatLng,
  sortByDistance,
  withinRadiusKm,
  initialBearingDeg,
  compassPoint8,
  compassPoint16,
  formatBearing,
  formatDistanceKm,
  unitSystemForRegion,
  encodeGeohash,
  decodeGeohash,
  geohashCellContains,
  type LatLng,
  type Ranked,
  type CompassPoint8,
  type CompassPoint16,
  type UnitSystem,
  type GeohashCell,
  type Range as GeoRange,
} from "./geo";

// schema
export {
  RECORD_TYPE_SCHEMAS,
  RABIES_VACCINATION,
  DOG_PROFILE,
  schemaFor,
  validateField,
  buildFieldsObject,
  type FieldDef,
  type FieldKind,
  type RecordTypeSchema,
} from "./schema/recordTypes";
export {
  isoDate,
  demoRabiesIssue,
  DEMO_ADMIN_PASSWORD,
  DEMO_OPERATOR_PASSWORD,
  DEMO_CUSTODY_PASSPHRASE,
  DEMO_RECORD_TYPE,
  DEMO_VERIFY_PURPOSES,
  DEMO_VACCINATION_DOCUMENT_STORE,
  DEMO_BUSINESS_VET,
  DEMO_BUSINESS_GROOMER,
  DEMO_BUSINESS_CONTACT_ONLY,
  DEMO_ISSUER_APPLICATION_VET,
  DEMO_ISSUER_APPLICATION_GROOMER,
  DEMO_WHITELIST_APPLY_VET,
  DEMO_WHITELIST_APPLY_GROOMER,
  DEMO_ADMIN_SIGNER,
  demoIssuerDeploy,
  DEMO_WHITELIST_GRANT,
  DEMO_PROVIDER_REGISTRATION,
  DEMO_CRM_CLIENT,
  DEMO_CRM_PET,
  DEMO_CRM_APPOINTMENT,
  DEMO_PROVIDER_LISTING,
  type DemoIssuerDeploy,
  type DemoWhitelistGrant,
  type DemoProviderRegistration,
  type DemoCrmClient,
  type DemoCrmPet,
  type DemoCrmAppointment,
  type DemoProviderListing,
  type DemoBusiness,
  type DemoIssuerApplication,
  type DemoWhitelistApply,
} from "./schema/demoData";

// provider self-service (registry-plan S-15)
export {
  ProviderSelfServiceFlows,
  type ProviderFlowCapabilities,
  type ProviderSelfServiceFlowsProps,
} from "./domain/ProviderSelfServiceFlows";
export {
  CloneLifecycleCard,
  DeployPlanCard,
  DirectoryPublicationCard,
  DomainClaimCard,
  ProviderCheckList,
  ProviderCheckRow,
  ProviderVerdictBadge,
} from "./domain/ProviderSelfServicePanel";
export {
  assessCandidateClone,
  assessDomainClaim,
  ATTACHMENT_IS_NOT_SELF_SERVICE,
  canWithdraw,
  CONTACT_ONLY_NOTICE,
  CONTACTS_ARE_ANCHORED_NOT_SERVED,
  COORDINATE_SCALE,
  createLiveProviderReader,
  describeDisposition,
  DomainDisposition,
  foldVerdict,
  MAX_DOMAIN_LENGTH,
  MAX_SCANNED_LOCATION_NUMBERS,
  mayContinueAfter,
  outcomeFromReceiptStatus,
  planCloneDeployment,
  planDirectoryPublication,
  providerCheck,
  readListingState,
  REPOINT_SCOPE_NOTICE,
  sendExplorerHref,
  sendRecord,
  sendStateLabel,
  Standing,
  STANDING_LABEL,
  toContractCoordinate,
  validateDomain,
  WITHDRAW_LOCATION_NOTICE,
  type CloneAssessment,
  type CloneLifecycle,
  type DeployPlan,
  type DirectoryListingState,
  type DirectoryPin,
  type DirectoryPublicationPlan,
  type DirectoryStep,
  type DomainClaimAssessment,
  type ProviderChainReader,
  type ProviderCheck,
  type ProviderContracts,
  type ProviderVerdict,
  type SendRecord,
  type SendState,
} from "./provider";

// The S-17 content-addressed profile and logo mirror. The serving half is `indexer-api`'s
// `src/mirror.rs`; this is the client that fetches and the recomputation that decides whether what
// came back is what was published.
export {
  buildProfileBlob,
  isContentAddress,
  keccakBytes,
  logoRef,
  mirrorContentReader,
  MULTIHASH_KECCAK_256,
  namesContent,
  parseProfileBlob,
  PROFILE_MEDIA_TYPE,
  ProviderLogo,
  PROVIDER_PROFILE_SCHEMA,
  PROVIDER_PROFILE_SCHEMA_ID,
  publicationDigest,
  putMirrorContent,
  resolveProviderProfile,
  SERVABLE_IMAGE_MEDIA_TYPES,
  checkLogoPublication,
  MAX_CONTENT_BYTES,
  verifyContentAddress,
  ZERO_ADDRESS,
  type BytesDigestFn,
  type ContentAddress,
  type ContentVerification,
  type FetchContentFn,
  type LogoPublication,
  type LogoState,
  type MirrorFetch,
  type ProfileAnchorRef,
  type ProfileBlobParse,
  type ProfileLogoRef,
  type ProfileResolution,
  type ProviderLogoProps,
  type ProviderProfile,
  type ServableImageMediaType,
  type LogoCheck,
} from "./mirror";
