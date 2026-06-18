import type { ComponentType, ReactNode } from "react";
import { cn } from "../lib/cn";

export interface NavItem {
  /** stable key + route target (the host wires `renderLink` to its router) */
  key: string;
  href: string;
  label: string;
  icon?: ComponentType<{ className?: string }>;
}

export interface AppShellProps {
  /** brand block at the top of the sidebar */
  brand: ReactNode;
  nav: NavItem[];
  activeKey?: string;
  /** host-provided link renderer (e.g. react-router <Link>); falls back to <a> */
  renderLink?: (item: NavItem, className: string, children: ReactNode) => ReactNode;
  /** card pinned to the bottom of the sidebar (user identity) */
  sidebarFooter?: ReactNode;
  /** sticky top bar inside the content column */
  topbar?: ReactNode;
  children: ReactNode;
}

/**
 * Dark sidebar + light content shell (impl §5.0 / groomer reference).
 * Colors come from semantic sidebar tokens so it stays dark-navy in both themes.
 */
export function AppShell({
  brand,
  nav,
  activeKey,
  renderLink,
  sidebarFooter,
  topbar,
  children,
}: AppShellProps) {
  const linkClass = (active: boolean) =>
    cn(
      "flex items-center gap-3 rounded-md px-3 py-2.5 text-sm font-medium transition-colors",
      active
        ? "bg-sidebar-active text-onSidebar"
        : "text-onSidebarMuted hover:bg-sidebar-muted hover:text-onSidebar",
    );

  return (
    <div className="flex min-h-screen bg-background">
      <aside className="sticky top-0 hidden h-screen w-64 shrink-0 flex-col bg-sidebar md:flex">
        <div className="px-5 py-6">{brand}</div>
        <nav className="flex-1 space-y-1 px-3">
          {nav.map((item) => {
            const active = item.key === activeKey;
            const Icon = item.icon;
            const inner = (
              <>
                {Icon && <Icon className="h-4 w-4 shrink-0" />}
                <span className="truncate">{item.label}</span>
              </>
            );
            const cls = linkClass(active);
            return renderLink ? (
              <span key={item.key}>{renderLink(item, cls, inner)}</span>
            ) : (
              <a key={item.key} href={item.href} className={cls}>
                {inner}
              </a>
            );
          })}
        </nav>
        {sidebarFooter && (
          <div className="border-t border-sidebar-muted p-4">{sidebarFooter}</div>
        )}
      </aside>

      <div className="flex min-w-0 flex-1 flex-col">
        {topbar && (
          <header className="sticky top-0 z-30 flex h-16 items-center justify-between gap-4 border-b border-border bg-surface/80 px-6 backdrop-blur">
            {topbar}
          </header>
        )}
        <main className="flex-1 px-4 py-6 sm:px-6 lg:px-8">{children}</main>
      </div>
    </div>
  );
}
