import { AppShell, ThemeToggle, WalletButton, type NavItem } from "@dogtag/ui";
import {
  CalendarDays,
  Download,
  LayoutDashboard,
  FileSignature,
  ListChecks,
  LogOut,
  Megaphone,
  Scissors,
  Settings as SettingsIcon,
  ShieldCheck,
  BarChart3,
  Users,
  Wand2,
} from "lucide-react";
import type { ReactNode } from "react";
import { Link, useLocation } from "react-router-dom";
import { useApp } from "./AppContext";

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
  { key: "issue", href: "/issue", label: "Issue credential", icon: FileSignature },
  { key: "verify", href: "/verify", label: "Verify", icon: ShieldCheck },
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
      <div className="mx-auto max-w-5xl">{children}</div>
    </AppShell>
  );
}
