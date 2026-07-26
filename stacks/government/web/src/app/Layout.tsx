import { AppShell, Badge, ThemeToggle, type NavItem } from "@dogtag/ui";
import { FilePlus2, Landmark, ListChecks, ShieldCheck, Waypoints } from "lucide-react";
import type { ReactNode } from "react";
import { Link, useLocation } from "react-router-dom";
import type { Health } from "../lib/api";

const NAV: NavItem[] = [
  { key: "issue", href: "/issue", label: "Issue", icon: FilePlus2 },
  { key: "verify", href: "/verify", label: "Verify", icon: ShieldCheck },
  { key: "records", href: "/records", label: "Records", icon: ListChecks },
  { key: "oversight", href: "/oversight", label: "Oversight", icon: Waypoints },
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

/** Which chain surface the backend is on, as far as this browser actually KNOWS.
 *
 *  THREE states, not two. `health` is `null` on first paint and whenever `/health` fails (App.tsx
 *  resets it in its `.catch`), and a two-state boolean collapses that unknown into "live" - so the UI
 *  asserted a green LIVE CHAIN with zero information, which is the same over-claiming this stack's
 *  `/health` fix removed. One helper feeds both the topbar badge and the sidebar strip so they cannot
 *  drift apart. Reads `simulated`, falling back to `backend`, so the two health fields cannot disagree. */
type ChainState = "live" | "simulated" | "unknown";

function chainState(health: Health | null): ChainState {
  if (!health) return "unknown";
  if (health.simulated ?? health.backend === "simulated") return "simulated";
  if (health.backend === "live" || typeof health.chainId === "number") return "live";
  return "unknown";
}

/** Ambient "who am I operating as" strip pinned to the bottom of the sidebar — promotes the buried
 *  chain identity (issuer / chainId / signer / signing capability) into constant context. */
function SidebarFooter({ health }: { health: Health | null }) {
  const state = chainState(health);
  const signer = state === "simulated" ? health?.simulatedSigner : health?.signer;
  return (
    <div className="space-y-1 text-xs text-onSidebarMuted">
      <div className="font-medium text-onSidebar">
        {state === "simulated"
          ? "SIMULATED chain · not a real network"
          : state === "unknown"
            ? "chain unknown · backend unreachable"
            : `ROAX · chainId ${health?.chainId ?? "?"}`}
      </div>
      <div className="truncate" title={signer ?? undefined}>
        {state === "simulated" ? "stand-in signer" : "signer"}{" "}
        {signer ? `${signer.slice(0, 10)}…` : "none"}
      </div>
      <div>
        {state === "simulated"
          ? "emulated only - nothing is broadcast"
          : state === "unknown"
            ? "capability unknown"
            : health?.canSign
              ? "can anchor on-chain"
              : "read-only (no signer)"}
      </div>
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
  const chain = chainState(health);
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
            {/* Two INDEPENDENT facts, deliberately two badges. The old single DEMO/LIVE badge was
                driven by `demo` (an ephemeral-store flag) and so read "DEMO" on a stack that was on
                the real chain, and could read "LIVE" on a simulated one. The chain badge is the one
                that matters for trusting a verdict, so it leads - and it never claims LIVE from an
                unanswered /health. */}
            <Badge
              variant={
                chain === "simulated" ? "danger" : chain === "unknown" ? "neutral" : "success"
              }
            >
              {chain === "simulated"
                ? "SIMULATED CHAIN"
                : chain === "unknown"
                  ? "CHAIN UNKNOWN"
                  : "LIVE CHAIN"}
            </Badge>
            {health?.demo ? <Badge variant="neutral">DEMO DATA</Badge> : null}
          </div>
          <ThemeToggle />
        </>
      }
    >
      <div className="mx-auto max-w-6xl">{children}</div>
    </AppShell>
  );
}
