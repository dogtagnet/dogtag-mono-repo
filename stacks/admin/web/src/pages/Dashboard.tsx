import {
  Badge,
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
  Spinner,
  explorerAddressUrl,
  useToast,
  type ActivityEvent,
  type ActivityStats,
  type ApiError,
  type AuthorityRole,
  type GovernanceAuthority,
  type IndexerStatus,
} from "@dogtag/ui";
import {
  Activity as ActivityIcon,
  BadgeCheck,
  Ban,
  Blocks,
  Building2,
  CheckCircle2,
  ExternalLink,
  ListChecks,
  Rocket,
  ScrollText,
  ShieldCheck,
  Users,
} from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { Link } from "react-router-dom";
import { useApp } from "../app/AppContext";
import { absoluteTime, eventMeta, relativeTime } from "../lib/activity";
import { shortAddr } from "../lib/format";

const RECENT_LIMIT = 8;

/**
 * Admin dashboard (plan §3.1). Fuses the registry/application counts with the PR-B oversight-indexer
 * aggregates: cross-issuer active/revoked credential counts, verifications, deployed clones, and a
 * chain-health card (indexer watermark vs chain head + finality + the live on-chain authority map).
 * A compact recent-activity slice links through to the full Activity feed. Every on-chain surface
 * degrades to a muted note when the indexer is not configured (503) - the registry counts keep working.
 */
export function Dashboard() {
  const { central } = useApp();
  const { toast } = useToast();

  const [businesses, setBusinesses] = useState<number | null>(null);
  const [pending, setPending] = useState<number | null>(null);
  const [approved, setApproved] = useState<number | null>(null);

  const [stats, setStats] = useState<ActivityStats | null>(null);
  const [status, setStatus] = useState<IndexerStatus | null>(null);
  const [authority, setAuthority] = useState<GovernanceAuthority | null>(null);
  const [recent, setRecent] = useState<ActivityEvent[] | null>(null);
  const [indexerReady, setIndexerReady] = useState<boolean | null>(null);

  const now = useMemo(() => Date.now(), [recent]);

  useEffect(() => {
    let cancelled = false;
    void (async () => {
      // registry counts (always available; independent of the indexer).
      try {
        const b = await central.listBusinesses();
        if (!cancelled) setBusinesses(b.businesses.length);
      } catch {
        if (!cancelled) setBusinesses(0);
      }
      try {
        const a = await central.listApplications();
        if (!cancelled) {
          setPending(a.applications.filter((x) => x.status === "pending").length);
          setApproved(a.applications.filter((x) => x.status === "approved").length);
        }
      } catch {
        if (!cancelled) {
          setPending(0);
          setApproved(0);
        }
      }

      // on-chain aggregates + recent feed (PR-B). A 503 means the indexer is unconfigured - render the
      // "not connected" state rather than an error toast.
      try {
        const [s, feed] = await Promise.all([
          central.getActivityStats(),
          central.getActivity({ limit: RECENT_LIMIT }),
        ]);
        if (!cancelled) {
          setStats(s);
          setRecent(feed.events);
          setIndexerReady(true);
        }
      } catch (err) {
        const e = err as ApiError;
        if (!cancelled) {
          setIndexerReady(false);
          setStats(null);
          setRecent([]);
          if (e.status !== 503) {
            toast({ title: "Failed to load activity", description: e.message, variant: "danger" });
          }
        }
      }

      // chain-health inputs: the indexer watermark + the live authority map. Best-effort and
      // independent - each renders "-" on failure without blocking the rest of the card.
      try {
        const st = await central.getActivityStatus();
        if (!cancelled) setStatus(st);
      } catch {
        /* indexer status unavailable - chain-health card shows what it can */
      }
      try {
        const auth = await central.getGovernanceAuthority();
        if (!cancelled) setAuthority(auth);
      } catch {
        /* authority map unavailable - chain-health card shows what it can */
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [central, toast]);

  return (
    <div className="space-y-6">
      {/* registry counts */}
      <div className="grid gap-4 sm:grid-cols-3">
        <Stat icon={Building2} label="Registered businesses" value={businesses} />
        <Stat icon={ListChecks} label="Pending applications" value={pending} />
        <Stat icon={BadgeCheck} label="Approved issuers" value={approved} />
      </div>

      {/* cross-issuer on-chain aggregates */}
      <section className="space-y-3">
        <h2 className="flex items-center gap-2 text-sm font-semibold text-onSurface">
          <ActivityIcon className="h-4 w-4 text-primary" /> Cross-issuer on-chain totals
        </h2>
        {indexerReady === false ? (
          <IndexerUnavailable />
        ) : (
          <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-5">
            <Stat
              icon={CheckCircle2}
              label="Active credentials"
              value={stats ? stats.activeCredentials : null}
              tone="success"
            />
            <Stat icon={Ban} label="Revoked" value={stats ? stats.rootRevoked : null} tone="danger" />
            <Stat icon={ShieldCheck} label="Verifications" value={stats ? stats.verifications : null} />
            <Stat icon={Rocket} label="Deployed issuers" value={stats ? stats.clones : null} />
            <Stat icon={Users} label="Whitelisted signers" value={stats ? stats.signers : null} />
          </div>
        )}
      </section>

      {/* chain-health + recent activity */}
      <div className="grid gap-6 lg:grid-cols-2">
        <ChainHealthCard status={status} authority={authority} indexerReady={indexerReady} />
        <RecentActivityCard events={recent} now={now} indexerReady={indexerReady} />
      </div>
    </div>
  );
}

function IndexerUnavailable() {
  return (
    <div className="rounded-md border border-dashed border-border bg-surface-muted p-4 text-sm text-muted">
      The oversight indexer is not connected in this environment, so cross-issuer on-chain totals are
      unavailable. Set <code>INDEXER_API_BASE</code> on the central backend to enable them.
    </div>
  );
}

/** Chain-health: indexer watermark vs chain head + finality + the live on-chain authority map. */
function ChainHealthCard({
  status,
  authority,
  indexerReady,
}: {
  status: IndexerStatus | null;
  authority: GovernanceAuthority | null;
  indexerReady: boolean | null;
}) {
  const lag = status?.lag ?? null;
  const lagTone = lag === null ? "neutral" : lag <= 5 ? "success" : lag <= 50 ? "warning" : "danger";
  const chainId = status?.chainId ?? authority?.chainId ?? null;

  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex items-center gap-2">
          <Blocks className="h-5 w-5 text-primary" /> Chain health
        </CardTitle>
        <CardDescription>
          ROAX{chainId ? ` (chain ${chainId})` : ""} indexing watermark and the live on-chain authority
          map.
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-4">
        {indexerReady === false && !status ? (
          <p className="text-sm text-muted">
            Indexer offline - no watermark available. The authority map below reads directly from chain.
          </p>
        ) : (
          <dl className="grid grid-cols-2 gap-3 text-sm">
            <Metric label="Chain head" value={status ? blockText(status.headBlock) : null} />
            <Metric label="Indexed block" value={status ? blockText(status.lastFinalizedIndexed) : null} />
            <Metric label="Finalized block" value={status ? blockText(status.finalizedBlock) : null} />
            <div>
              <dt className="text-xs text-muted">Indexing lag</dt>
              <dd className="mt-0.5">
                {lag === null ? (
                  <span className="text-muted">-</span>
                ) : (
                  <Badge variant={lagTone}>{lag} block{lag === 1 ? "" : "s"} behind</Badge>
                )}
              </dd>
            </div>
            {status && (
              <div className="col-span-2">
                <dt className="text-xs text-muted">Finality source</dt>
                <dd className="mt-0.5">
                  <Badge variant={status.finalitySource === "finalized-tag" ? "success" : "neutral"}>
                    {status.finalitySource === "finalized-tag"
                      ? "chain finality tag"
                      : status.finalitySource === "confirmations-fallback"
                        ? `${status.confirmations}-confirmation depth`
                        : "unknown"}
                  </Badge>
                </dd>
              </div>
            )}
          </dl>
        )}

        <div className="space-y-2 border-t border-border pt-4">
          <h3 className="text-xs font-semibold uppercase tracking-wide text-muted">Authority</h3>
          {authority === null ? (
            <p className="text-sm text-muted">Authority map unavailable.</p>
          ) : (
            <div className="space-y-2">
              <AuthorityRow
                label="Factory owner"
                role={authority.factoryOwner}
                holder={authority.factoryOwner.owner}
              />
              <AuthorityRow
                label="Whitelist admin"
                role={authority.whitelistAdmin}
                holder={authority.hostedSigner}
                holderIsHostedFallback
              />
              <AuthorityRow
                label="Default admin"
                role={authority.defaultAdmin}
                holder={authority.defaultAdmin.holder}
              />
            </div>
          )}
        </div>
      </CardContent>
    </Card>
  );
}

/** One authority role: who holds it, whether the hosted key holds it, and any pending transfer. */
function AuthorityRow({
  label,
  role,
  holder,
  holderIsHostedFallback,
}: {
  label: string;
  role: AuthorityRole;
  holder?: string | null;
  holderIsHostedFallback?: boolean;
}) {
  const pending = role.pendingTransfer ?? null;
  const pendingOwner = role.pendingOwner ?? null;
  return (
    <div className="flex items-start justify-between gap-3 rounded-md bg-surface-muted px-3 py-2">
      <div className="min-w-0">
        <div className="text-sm font-medium text-onSurface">{label}</div>
        {holder ? (
          <a
            href={explorerAddressUrl(holder)}
            target="_blank"
            rel="noreferrer"
            className="font-mono text-xs text-primary hover:underline"
          >
            {shortAddr(holder)}
          </a>
        ) : holderIsHostedFallback && role.heldByHosted ? (
          <span className="text-xs text-muted">hosted operator key</span>
        ) : (
          <span className="text-xs text-muted">-</span>
        )}
        {(pending || pendingOwner) && (
          <div className="mt-1 text-xs text-warning">
            Pending transfer → {shortAddr(pending?.newAdmin ?? pendingOwner ?? "")}
            {pending?.acceptSchedule ? ` (ETA ${absoluteTime(pending.acceptSchedule)})` : ""}
          </div>
        )}
      </div>
      <div className="shrink-0">
        {role.heldByHosted === true ? (
          <Badge variant="success">hosted</Badge>
        ) : role.heldByHosted === false ? (
          <Badge variant="neutral">governance</Badge>
        ) : (
          <Badge variant="neutral">-</Badge>
        )}
      </div>
    </div>
  );
}

function Metric({ label, value }: { label: string; value: string | null }) {
  return (
    <div>
      <dt className="text-xs text-muted">{label}</dt>
      <dd className="mt-0.5 font-mono text-sm text-onSurface">
        {value === null ? <span className="text-muted">-</span> : value}
      </dd>
    </div>
  );
}

function blockText(n: number | null): string {
  return n === null || n === undefined ? "-" : `#${n.toLocaleString()}`;
}

/** A compact slice of the newest on-chain events with a link through to the full Activity feed. */
function RecentActivityCard({
  events,
  now,
  indexerReady,
}: {
  events: ActivityEvent[] | null;
  now: number;
  indexerReady: boolean | null;
}) {
  return (
    <Card>
      <CardHeader className="flex flex-row items-start justify-between gap-4">
        <div>
          <CardTitle className="flex items-center gap-2">
            <ScrollText className="h-5 w-5 text-primary" /> Recent activity
          </CardTitle>
          <CardDescription>The latest cross-issuer on-chain events.</CardDescription>
        </div>
        <Link
          to="/activity"
          className="inline-flex items-center gap-1 text-sm font-medium text-primary hover:underline"
        >
          View all <ExternalLink className="h-3.5 w-3.5" />
        </Link>
      </CardHeader>
      <CardContent>
        {events === null ? (
          <div className="flex justify-center py-8">
            <Spinner className="h-5 w-5 text-muted" />
          </div>
        ) : indexerReady === false ? (
          <p className="py-8 text-center text-sm text-muted">Indexer not connected.</p>
        ) : events.length === 0 ? (
          <p className="py-8 text-center text-sm text-muted">No on-chain events yet.</p>
        ) : (
          <ul className="divide-y divide-border">
            {events.map((ev) => {
              const meta = eventMeta(ev.type);
              const who = ev.actorName ?? ev.cloneName ?? (ev.actor ? shortAddr(ev.actor) : null);
              return (
                <li key={ev.id} className="flex items-center justify-between gap-3 py-2">
                  <div className="flex min-w-0 items-center gap-2">
                    <Badge variant={meta.variant}>{meta.label}</Badge>
                    {who && <span className="truncate text-sm text-onSurface">{who}</span>}
                  </div>
                  <span
                    className="shrink-0 text-xs text-muted"
                    title={absoluteTime(ev.blockTimestamp)}
                  >
                    {relativeTime(ev.blockTimestamp, now)}
                  </span>
                </li>
              );
            })}
          </ul>
        )}
      </CardContent>
    </Card>
  );
}

function Stat({
  icon: Icon,
  label,
  value,
  tone,
}: {
  icon: typeof Building2;
  label: string;
  value: number | null;
  tone?: "success" | "danger";
}) {
  const valueColor =
    tone === "success" ? "text-success" : tone === "danger" ? "text-danger" : "text-onSurface";
  return (
    <Card>
      <CardContent className="flex items-center gap-4 pt-6">
        <span className="flex h-10 w-10 items-center justify-center rounded-lg bg-primary/10">
          <Icon className="h-5 w-5 text-primary" />
        </span>
        <div>
          <div className={`text-2xl font-semibold ${valueColor}`}>
            {value === null ? <Spinner className="h-5 w-5 text-muted" /> : value.toLocaleString()}
          </div>
          <div className="text-sm text-muted">{label}</div>
        </div>
      </CardContent>
    </Card>
  );
}
