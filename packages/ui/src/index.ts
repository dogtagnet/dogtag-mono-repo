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
export { createWalletConfig, roaxNetwork, type WalletConfigOptions } from "./wallet/config";
export {
  DEPLOYED_ADDRESSES,
  recordTypeKey,
  roaxPublicClient,
  isWhitelistedFor,
  isRootValid,
} from "./wallet/contracts";
export { WalletProvider } from "./wallet/WalletProvider";
export { WalletButton, shortAddress } from "./wallet/WalletButton";
export { useRoaxChain, type UseRoaxChainResult } from "./wallet/useRoaxChain";

// domain components
export { SigningModeToggle, type SigningModeToggleProps } from "./domain/SigningModeToggle";
export { StatusPanel, formatPlasma, type StatusPanelProps } from "./domain/StatusPanel";
export { VerifyFlow, type VerifyFlowProps, type VerifyPurpose } from "./domain/VerifyFlow";
export {
  IssuanceStatusPanel,
  type IssuanceStatusPanelProps,
} from "./domain/IssuanceStatusPanel";

// api
export { createApiClient, type ApiClient, type ApiClientOptions } from "./api/client";
export {
  createCentralClient,
  type CentralClient,
  type CentralClientOptions,
} from "./api/central";
export * from "./api/types";

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
