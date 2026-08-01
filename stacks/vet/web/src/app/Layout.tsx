import {
  AppShell,
  Button,
  CustodyLockedBanner,
  CustodyUnlockDialog,
  ThemeToggle,
  WalletButton,
  useToast,
  DEMO_ADMIN_PASSWORD,
  DEMO_CUSTODY_PASSPHRASE,
  type NavItem,
} from "@dogtag/ui";
import {
  Building2,
  Dog,
  FilePlus2,
  ListChecks,
  LogOut,
  Settings as SettingsIcon,
  ShieldCheck,
  Download,
  Wand2,
  Waypoints,
} from "lucide-react";
import type { ReactNode } from "react";
import { Link, useLocation } from "react-router-dom";
import { useApp } from "./AppContext";
import { env } from "../lib/env";

// REGISTER-FIRST (audit §7, re-implemented in M5): "Register pet" is listed BEFORE "Issue a record"
// because that is the actual order of operations - a record can only attach to a pet that already has a
// dog tag. The old labels ("Issue credential" / "Issue dog tag") read as two interchangeable ways to
// issue something and gave no hint that one is a prerequisite for the other.
const NAV: NavItem[] = [
  { key: "setup", href: "/setup", label: "Setup", icon: Wand2 },
  { key: "issue-dog-tag", href: "/issue-dog-tag", label: "Register pet", icon: Dog },
  { key: "issue", href: "/issue", label: "Issue a record", icon: FilePlus2 },
  { key: "records", href: "/records", label: "Records", icon: ListChecks },
  { key: "traceability", href: "/traceability", label: "Traceability", icon: Waypoints },
  { key: "import", href: "/import", label: "Import from user", icon: Download },
  { key: "verify", href: "/verify", label: "Verification", icon: ShieldCheck },
  { key: "provider", href: "/provider", label: "Provider self-service", icon: Building2 },
  { key: "settings", href: "/settings", label: "Settings", icon: SettingsIcon },
];

function Brand() {
  return (
    <Link to="/issue" className="flex items-center gap-2">
      <span className="flex h-9 w-9 items-center justify-center rounded-lg bg-primary text-onPrimary font-bold">
        DT
      </span>
      <span className="leading-tight">
        <span className="block font-semibold text-onSidebar">DogTag</span>
        <span className="block text-xs uppercase tracking-wide text-onSidebarMuted">Vet Portal</span>
      </span>
    </Link>
  );
}


/**
 * Hosts the point-of-need unlock prompt and the locked banner for every page inside the shell.
 *
 * The prompt is raised by the api client the moment a request is refused with "not unlocked", so the
 * operator unlocks WITHOUT leaving the page they are on: the refused request is replayed on success
 * and a half-filled form keeps every value. The banner covers the other case - arriving at an
 * already-locked backend - without redirecting anyone, because a front-desk operator who holds the
 * operator password but not the custody-admin password must still reach the read-only pages.
 */
function CustodyPrompt() {
  const {
    api,
    adminToken,
    setAdminToken,
    custodyState,
    unlockPromptOpen,
    resolveUnlockPrompt,
    openUnlockPrompt,
    setSignerAddress,
  } = useApp();
  const { toast } = useToast();
  return (
    <>
      {custodyState === "locked" && !unlockPromptOpen && (
        <CustodyLockedBanner onUnlock={openUnlockPrompt} />
      )}
      <CustodyUnlockDialog
        open={unlockPromptOpen}
        onDismiss={() => resolveUnlockPrompt(false)}
        demoMode={env.demoMode}
        demoAdminPassword={DEMO_ADMIN_PASSWORD}
        demoPassphrase={DEMO_CUSTODY_PASSPHRASE}
        adminLogin={api.adminLogin}
        unlock={(passphrase) => api.unlock({ passphrase })}
        adminToken={adminToken}
        onAdminToken={setAdminToken}
        onAlreadyUnlocked={() => resolveUnlockPrompt(true)}
        onUnlocked={(accounts) => {
          if (accounts[0]?.address) setSignerAddress(accounts[0].address);
          toast({ title: "Custody unlocked", variant: "success" });
          resolveUnlockPrompt(true);
        }}
        setupLink={
          <Button asChild className="w-full">
            <Link to="/setup">Go to Setup</Link>
          </Button>
        }
      />
    </>
  );
}

export function Layout({ children, title }: { children: ReactNode; title: string }) {
  const location = useLocation();
  const { logout } = useApp();
  // Longest matching href wins so /issue-dog-tag highlights its own item, not /issue.
  const activeKey = NAV.filter((n) => location.pathname.startsWith(n.href)).sort(
    (a, b) => b.href.length - a.href.length,
  )[0]?.key;

  return (
    <AppShell
      brand={<Brand />}
      nav={NAV}
      activeKey={activeKey}
      renderLink={(item, className, inner) => (
        <Link to={item.href} className={className}>
          {inner}
        </Link>
      )}
      sidebarFooter={
        <button
          type="button"
          onClick={logout}
          className="flex w-full items-center gap-2 rounded-md px-2 py-2 text-sm text-onSidebarMuted transition-colors hover:bg-sidebar-muted hover:text-onSidebar"
        >
          <LogOut className="h-4 w-4" />
          Sign out
        </button>
      }
      topbar={
        <>
          <h1 className="text-lg font-semibold text-onSurface">{title}</h1>
          <div className="flex items-center gap-3">
            <WalletButton defaultRpcUrl={env.roaxRpc} />
            <ThemeToggle />
          </div>
        </>
      }
    >
      <div className="mx-auto max-w-5xl">
        <CustodyPrompt />
        {children}
      </div>
    </AppShell>
  );
}
