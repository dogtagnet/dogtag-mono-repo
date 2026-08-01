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
  History,
  LayoutDashboard,
  ListChecks,
  LogOut,
  Megaphone,
  PawPrint,
  Scissors,
  Settings as SettingsIcon,
  ShieldCheck,
  BarChart3,
  Users,
  Wand2,
  Building2,
} from "lucide-react";
import type { ReactNode } from "react";
import { Link, useLocation } from "react-router-dom";
import { useApp } from "./AppContext";
import { env } from "../lib/env";

/**
 * The groomer's working nav (impl §5.2): the daily surfaces first (Calendar / Appointments /
 * Clients), then the verification history, then the supporting DogTag sections.
 *
 * There is NO "Issue credential" entry: a groomer verifies, it does not issue. The backend agrees —
 * `BUSINESS_TYPE=groomer` does not mount the issuance routes at all.
 */
const NAV: NavItem[] = [
  { key: "dashboard", href: "/dashboard", label: "Dashboard", icon: LayoutDashboard },
  { key: "calendar", href: "/calendar", label: "Calendar", icon: CalendarDays },
  { key: "appointments", href: "/appointments", label: "Appointments", icon: ListChecks },
  { key: "clients", href: "/clients", label: "Clients", icon: Users },
  { key: "pets", href: "/pets", label: "Pets", icon: PawPrint },
  { key: "verifications", href: "/verifications", label: "All verifications", icon: History },
  { key: "groomers", href: "/groomers", label: "Groomers", icon: Scissors },
  { key: "reports", href: "/reports", label: "Reports", icon: BarChart3 },
  { key: "marketing", href: "/marketing", label: "Marketing", icon: Megaphone },
  { key: "import", href: "/import", label: "Import from user", icon: Download },
  { key: "verify", href: "/verify", label: "Ad-hoc verification", icon: ShieldCheck },
  { key: "setup", href: "/setup", label: "Setup", icon: Wand2 },
  { key: "provider", href: "/provider", label: "Provider self-service", icon: Building2 },
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

/**
 * Which nav entry owns the current path. Matches on a full path SEGMENT, not a raw string prefix:
 * `/verifications` must not light up `/verify` just because one is a prefix of the other.
 */
function activeNavKey(pathname: string): string | undefined {
  return NAV.find((n) => pathname === n.href || pathname.startsWith(`${n.href}/`))?.key;
}

export function Layout({
  children,
  title,
  /**
   * Mark this page a WORKING SURFACE rather than something to read.
   *
   * The two kinds of page want opposite things. A form or a detail view is READ: a label/field pair
   * stretched across a 27" monitor is harder to follow, not easier, so those stay capped at a
   * comfortable measure and centred. A booking list or a calendar is WORKED IN: a six-column table,
   * a filter row and seven day-columns all need the width the screen actually has, and capping them
   * leaves the operator scanning a cramped table beside a band of empty background.
   *
   * So this is not a bigger cap — a working surface takes the CONTENT COLUMN ITSELF, whatever width
   * that is. The shell already supplies the gutters (`AppShell`'s `main` padding) and every table is
   * wrapped in its own horizontal-scroll container, so the same markup holds up at 3840px and at
   * 360px without a second layout.
   */
  wide = false,
}: {
  children: ReactNode;
  title: string;
  wide?: boolean;
}) {
  const location = useLocation();
  const { logout } = useApp();
  const activeKey = activeNavKey(location.pathname);

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
      <div className={wide ? "w-full" : "mx-auto w-full max-w-5xl"}>
        <CustodyPrompt />
        {children}
      </div>
    </AppShell>
  );
}
