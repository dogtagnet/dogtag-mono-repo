import { AppShell, ThemeToggle, type NavItem } from "@dogtag/ui";
import {
  LayoutDashboard,
  ListChecks,
  LogOut,
  ShieldCheck,
  Building2,
} from "lucide-react";
import type { ReactNode } from "react";
import { Link, useLocation } from "react-router-dom";
import { useApp } from "./AppContext";

const NAV: NavItem[] = [
  { key: "dashboard", href: "/dashboard", label: "Dashboard", icon: LayoutDashboard },
  { key: "businesses", href: "/businesses", label: "Business registry", icon: Building2 },
  { key: "applications", href: "/applications", label: "Issuer applications", icon: ListChecks },
  { key: "whitelist", href: "/whitelist", label: "Whitelist viewer", icon: ShieldCheck },
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
          Admin Portal
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
            <ThemeToggle />
          </div>
        </>
      }
    >
      <div className="mx-auto max-w-6xl">{children}</div>
    </AppShell>
  );
}
