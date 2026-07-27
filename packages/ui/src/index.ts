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

// wallet
export {
  roax,
  ROAX_CHAIN_ID,
  ROAX_CHAIN_ID_HEX,
  ROAX_ADD_CHAIN_PARAMS,
  explorerTxUrl,
  explorerAddressUrl,
} from "./wallet/chain";

// on-chain provenance (audit surfaces: government Oversight, vet/groomer Traceability)
export {
  isHash32,
  isEvmAddress,
  chainProvenance,
  txExplorerHref,
  addressExplorerHref,
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
  DEMO_ISSUER_APPLICATION_VET,
  DEMO_ISSUER_APPLICATION_GROOMER,
  DEMO_WHITELIST_APPLY_VET,
  DEMO_WHITELIST_APPLY_GROOMER,
  type DemoBusiness,
  type DemoIssuerApplication,
  type DemoWhitelistApply,
} from "./schema/demoData";
