import { AppShell, Badge, ThemeToggle, type NavItem } from "@dogtag/ui";
import { FilePlus2, Landmark, ListChecks, ShieldCheck } from "lucide-react";
import type { ReactNode } from "react";
import { Link, useLocation } from "react-router-dom";
import type { Health } from "../lib/api";

const NAV: NavItem[] = [
  { key: "issue", href: "/issue", label: "Issue", icon: FilePlus2 },
  { key: "verify", href: "/verify", label: "Verify", icon: ShieldCheck },
  { key: "records", href: "/records", label: "Records", icon: ListChecks },
];

function Brand() {
  return (
    <Link to="/issue" className="flex items-center gap-2">
      <span className="flex h-9 w-9 items-center justify-center rounded-lg bg-primary text-onPrimary">
        <Landmark className="h-5 w-5" />
      </span>
      <span className="leading-tight">
        <span className="block font-semibold text-onSidebar">DogTag</span>
        <span className="block text-xs uppercase tracking-wide text-onSidebarMuted">
          Government Portal
        </span>
      </span>
    </Link>
  );
}

/** Ambient "who am I operating as" strip pinned to the bottom of the sidebar — promotes the buried
 *  chain identity (issuer / chainId / signer / signing capability) into constant context. */
function SidebarFooter({ health }: { health: Health | null }) {
  const signer = health?.signer;
  return (
    <div className="space-y-1 text-xs text-onSidebarMuted">
      <div className="font-medium text-onSidebar">
        ROAX · chainId {health?.chainId ?? "?"}
      </div>
      <div className="truncate" title={signer ?? undefined}>
        signer {signer ? `${signer.slice(0, 10)}…` : "none"}
      </div>
      <div>{health?.canSign ? "can anchor on-chain" : "read-only (no signer)"}</div>
    </div>
  );
}

export function Layout({
  children,
  title,
  health,
}: {
  children: ReactNode;
  title: string;
  health: Health | null;
}) {
  const location = useLocation();
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
      sidebarFooter={<SidebarFooter health={health} />}
      topbar={
        <>
          <div className="flex items-center gap-3">
            <h1 className="text-lg font-semibold text-onSurface">{title}</h1>
            <Badge variant={health?.demo ? "warning" : "success"}>
              {health?.demo ? "DEMO" : "LIVE"}
            </Badge>
          </div>
          <ThemeToggle />
        </>
      }
    >
      <div className="mx-auto max-w-6xl">{children}</div>
    </AppShell>
  );
}
