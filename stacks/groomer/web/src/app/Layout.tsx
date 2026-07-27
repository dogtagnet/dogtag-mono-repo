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
  CalendarDays,
  Download,
  LayoutDashboard,
  FileSignature,
  ListChecks,
  FileStack,
  LogOut,
  Megaphone,
  Scissors,
  Settings as SettingsIcon,
  ShieldCheck,
  BarChart3,
  Users,
  Wand2,
  Waypoints,
} from "lucide-react";
import type { ReactNode } from "react";
import { Link, useLocation } from "react-router-dom";
import { useApp } from "./AppContext";
import { env } from "../lib/env";

/**
 * Mirrors the reference groomer dashboard nav (impl §5.2):
 * Dashboard / Calendar / Appointments / Clients / Groomers / Reports / Marketing — plus the
 * DogTag-specific Import / Verify / Setup / Settings sections.
 */
const NAV: NavItem[] = [
  { key: "dashboard", href: "/dashboard", label: "Dashboard", icon: LayoutDashboard },
  { key: "calendar", href: "/calendar", label: "Calendar", icon: CalendarDays },
  { key: "appointments", href: "/appointments", label: "Appointments", icon: ListChecks },
  { key: "clients", href: "/clients", label: "Clients", icon: Users },
  { key: "groomers", href: "/groomers", label: "Groomers", icon: Scissors },
  { key: "reports", href: "/reports", label: "Reports", icon: BarChart3 },
  { key: "marketing", href: "/marketing", label: "Marketing", icon: Megaphone },
  { key: "import", href: "/import", label: "Import from user", icon: Download },
  { key: "issue", href: "/issue", label: "Issue a record", icon: FileSignature },
  { key: "records", href: "/records", label: "Records", icon: FileStack },
  { key: "traceability", href: "/traceability", label: "Traceability", icon: Waypoints },
  { key: "verify", href: "/verify", label: "Verification", icon: ShieldCheck },
  { key: "setup", href: "/setup", label: "Setup", icon: Wand2 },
  { key: "settings", href: "/settings", label: "Settings", icon: SettingsIcon },
];

function Brand() {
  return (
    <Link to="/dashboard" className="flex items-center gap-2">
      <span className="flex h-9 w-9 items-center justify-center rounded-lg bg-primary text-onPrimary font-bold">
        DT
      </span>
      <span className="leading-tight">
        <span className="block font-semibold text-onSidebar">DogTag</span>
        <span className="block text-xs uppercase tracking-wide text-onSidebarMuted">
          Groomer Portal
        </span>
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
  const activeKey = NAV.find((n) => location.pathname.startsWith(n.href))?.key;

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
            <WalletButton />
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
