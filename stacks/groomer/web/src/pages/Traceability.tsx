import {
  Badge,
  Button,
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
  explorerAddressUrl,
  shortAddress,
  useToast,
  type ApiError,
  type TraceActivityResp,
  type TraceEvent,
  type TraceEventType,
  type TraceStatsResp,
} from "@dogtag/ui";
import { RefreshCw, ShieldCheck } from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { useApp } from "../app/AppContext";

/** Human label + badge colour per on-chain event kind (mirrors the admin Activity console vocab). */
const EVENT_META: Record<
  TraceEventType,
  { label: string; variant: "default" | "neutral" | "success" | "warning" | "danger" }
> = {
  issuerCreated: { label: "Issuer created", variant: "default" },
  rootRegistered: { label: "Root registered", variant: "neutral" },
  whitelisted: { label: "Whitelisted", variant: "success" },
  delisted: { label: "Delisted", variant: "warning" },
  rootIssued: { label: "Root issued", variant: "success" },
  rootRevoked: { label: "Root revoked", variant: "danger" },
  verified: { label: "Verified", variant: "default" },
};

const ALL = "all";
const TYPE_OPTIONS: TraceEventType[] = [
  "rootIssued",
  "rootRevoked",
  "verified",
  "whitelisted",
  "delisted",
  "issuerCreated",
];

function eventMeta(t: string) {
  return EVENT_META[t as TraceEventType] ?? { label: t, variant: "neutral" as const };
}

/** Format a Unix-seconds timestamp as a compact local date-time; "—" when absent. */
function fmtTime(secs?: number): string {
  if (!secs) return "—";
  return new Date(secs * 1000).toLocaleString();
}

/** Shorten a long opaque identifier (the field-hashed decimal dogTagId, a hashed purpose key). */
function shortId(id: string): string {
  return id.length > 12 ? `${id.slice(0, 6)}…${id.slice(-4)}` : id;
}

/** Human label for the joined local row's kind. */
function localKindLabel(kind?: string): string {
  if (kind === "verification") return "verification";
  if (kind === "mint") return "dog tag";
  return "record";
}

function txLink(ev: TraceEvent): string | null {
  return ev.txUrl ?? null;
}

/** One stat tile in the summary strip. */
function Stat({ label, value, testId }: { label: string; value: number | string; testId?: string }) {
  return (
    <div className="rounded-lg border border-border bg-surface-muted px-3 py-2">
      <div className="text-lg font-semibold text-onSurface" data-testid={testId}>
        {value}
      </div>
      <div className="text-[11px] uppercase tracking-wide text-muted">{label}</div>
    </div>
  );
}

export function Traceability() {
  const { api } = useApp();
  const { toast } = useToast();
  const [resp, setResp] = useState<TraceActivityResp | null>(null);
  const [stats, setStats] = useState<TraceStatsResp | null>(null);
  const [loading, setLoading] = useState(true);
  const [unavailable, setUnavailable] = useState<string | null>(null);
  const [type, setType] = useState<string>(ALL);
  const [finality, setFinality] = useState<string>(ALL);

  const load = useCallback(async () => {
    setLoading(true);
    setUnavailable(null);
    try {
      const [a, s] = await Promise.all([
        api.traceActivity({
          type: type === ALL ? undefined : (type as TraceEventType),
          finality: finality === ALL ? undefined : (finality as "finalized" | "pending"),
          limit: 100,
        }),
        api.traceStats().catch(() => null),
      ]);
      setResp(a);
      setStats(s);
    } catch (err) {
      const e = err as ApiError;
      if (e.status === 503) {
        // The oversight indexer is not connected — a first-class empty state, not an error toast.
        setUnavailable(e.message);
        setResp(null);
        setStats(null);
      } else {
        toast({ title: "Failed to load activity", description: e.message, variant: "danger" });
      }
    } finally {
      setLoading(false);
    }
  }, [api, toast, type, finality]);

  useEffect(() => {
    void load();
  }, [load]);

  const events = resp?.events ?? [];
  const scopeLabel = resp?.scope?.label;

  return (
    <Card>
      <CardHeader>
        <div className="flex flex-wrap items-start justify-between gap-2">
          <div>
            <CardTitle className="flex items-center gap-2">
              <ShieldCheck className="h-5 w-5 text-primary" /> Traceability
            </CardTitle>
            <CardDescription>
              The on-chain credential activity for the roots this business issued and handled - scoped
              server-side to your own signer(s) and issuer clone(s), and joined to your own records.
              You never see another operator's activity.
            </CardDescription>
          </div>
          <Button
            size="sm"
            variant="outline"
            data-testid="trace-refresh"
            onClick={() => void load()}
            loading={loading}
          >
            <RefreshCw className="h-4 w-4" /> Refresh
          </Button>
        </div>
      </CardHeader>

      <CardContent>
        {unavailable ? (
          <div
            className="rounded-lg border border-dashed border-border bg-surface-muted px-4 py-8 text-center"
            data-testid="trace-unavailable"
          >
            <p className="text-sm font-medium text-onSurface">Oversight indexer not connected</p>
            <p className="mx-auto mt-1 max-w-md text-xs text-muted">
              The traceability feed reads on-chain events from the oversight indexer. It is not
              configured for this deployment yet (set <code>INDEXER_API_BASE</code>). Your records
              remain available on the Records page.
            </p>
          </div>
        ) : (
          <>
            {/* Summary strip: in-scope + join reconciliation + scoped chain counters. The dog-tag
                tile only appears once this operator has actually minted (groomers never do). */}
            <div
              className={`mb-4 grid grid-cols-2 gap-2 sm:grid-cols-3 ${
                (stats?.local?.dogTagsMinted ?? 0) > 0 ? "lg:grid-cols-6" : "lg:grid-cols-5"
              }`}
            >
              <Stat label="In scope" value={resp?.inScope ?? events.length} testId="trace-inscope" />
              <Stat label="Matched to a record" value={resp?.matched ?? 0} testId="trace-matched" />
              <Stat label="Issued" value={stats?.rootIssued ?? 0} />
              <Stat label="Revoked" value={stats?.rootRevoked ?? 0} />
              <Stat label="Verifications" value={stats?.verifications ?? 0} />
              {(stats?.local?.dogTagsMinted ?? 0) > 0 && (
                <Stat
                  label="Dog tags minted"
                  value={stats?.local?.dogTagsMinted ?? 0}
                  testId="trace-mints"
                />
              )}
            </div>

            {scopeLabel && (
              <p className="mb-3 text-xs text-muted">
                Scope: <span className="font-medium text-onSurface">{scopeLabel}</span>
                {resp?.localScope
                  ? ` · ${resp.localScope.signers} signer(s), ${resp.localScope.clones} clone(s)`
                  : ""}
                {resp?.droppedOutOfScope
                  ? ` · ${resp.droppedOutOfScope} out-of-scope event(s) filtered server-side`
                  : ""}
              </p>
            )}

            {/* Filters */}
            <div className="mb-4 flex flex-wrap gap-3">
              <div className="w-44">
                <Select value={type} onValueChange={setType}>
                  <SelectTrigger data-testid="trace-filter-type">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value={ALL}>All events</SelectItem>
                    {TYPE_OPTIONS.map((t) => (
                      <SelectItem key={t} value={t}>
                        {eventMeta(t).label}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>
              <div className="w-40">
                <Select value={finality} onValueChange={setFinality}>
                  <SelectTrigger data-testid="trace-filter-finality">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value={ALL}>Any finality</SelectItem>
                    <SelectItem value="finalized">Finalized</SelectItem>
                    <SelectItem value="pending">Pending</SelectItem>
                  </SelectContent>
                </Select>
              </div>
            </div>

            {events.length === 0 ? (
              <p className="py-8 text-center text-sm text-muted" data-testid="trace-empty">
                {loading ? "Loading…" : "No on-chain activity in scope yet."}
              </p>
            ) : (
              <div className="overflow-x-auto">
                <Table>
                  <TableHeader>
                    <TableRow>
                      <TableHead>Event</TableHead>
                      <TableHead>Details</TableHead>
                      <TableHead>Your record</TableHead>
                      <TableHead>Signer</TableHead>
                      <TableHead>Block</TableHead>
                      <TableHead>When</TableHead>
                      <TableHead>Tx</TableHead>
                    </TableRow>
                  </TableHeader>
                  <TableBody>
                    {events.map((ev) => {
                      const meta = eventMeta(ev.type);
                      const link = txLink(ev);
                      return (
                        <TableRow key={ev.id} data-testid="trace-event-row" data-type={ev.type}>
                          <TableCell>
                            <Badge variant={meta.variant}>{meta.label}</Badge>
                          </TableCell>
                          <TableCell className="text-xs" data-testid="trace-details">
                            <div className="flex flex-col gap-0.5">
                              {(ev.recordType ?? ev.local?.recordType) ? (
                                <span>{ev.recordType ?? ev.local?.recordType}</span>
                              ) : ev.type !== "verified" ? (
                                <span className="text-muted">—</span>
                              ) : null}
                              {/* Owner-blind verified payload: opaque tag id, hashed purpose key
                                  (the joined session names it), proof-bound consent deadline. */}
                              {ev.type === "verified" && (
                                <>
                                  {ev.dogTagId && (
                                    <span
                                      className="font-mono text-[10px] text-muted"
                                      title={ev.dogTagId}
                                    >
                                      tag {shortId(ev.dogTagId)}
                                    </span>
                                  )}
                                  <span className="font-mono text-[10px] text-muted" title={ev.purpose}>
                                    purpose {ev.local?.purpose ?? (ev.purpose ? shortId(ev.purpose) : "—")}
                                  </span>
                                  {(ev.deadline ?? 0) > 0 && (
                                    <span className="text-[10px] text-muted">
                                      consent until {fmtTime(ev.deadline)}
                                    </span>
                                  )}
                                </>
                              )}
                            </div>
                          </TableCell>
                          <TableCell data-testid="trace-local">
                            {ev.local ? (
                              <div className="flex flex-col gap-0.5">
                                <Badge variant="success" data-testid="trace-local-matched">
                                  {localKindLabel(ev.local.kind)}
                                </Badge>
                                <span className="font-mono text-[10px] text-muted">
                                  {ev.local.recordId ?? ev.local.sessionId}
                                  {ev.local.dogTagId ? ` · tag ${ev.local.dogTagId}` : ""}
                                  {ev.local.status ? ` · ${ev.local.status}` : ""}
                                </span>
                                {ev.local.label && (
                                  <span className="max-w-[10rem] truncate text-[10px] text-muted">
                                    {ev.local.label}
                                  </span>
                                )}
                              </div>
                            ) : (
                              <span className="text-[11px] text-muted" data-testid="trace-local-none">
                                on-chain only
                              </span>
                            )}
                          </TableCell>
                          <TableCell className="text-xs">
                            {ev.actor ? (
                              <a
                                className="font-mono text-primary hover:underline"
                                href={explorerAddressUrl(ev.actor)}
                                target="_blank"
                                rel="noreferrer"
                                title={ev.actorName ?? ev.actor}
                              >
                                {ev.actorName ?? shortAddress(ev.actor)}
                              </a>
                            ) : (
                              <span className="text-muted">—</span>
                            )}
                          </TableCell>
                          <TableCell className="font-mono text-xs">
                            <div className="flex flex-col">
                              <span>#{ev.blockNumber ?? "?"}</span>
                              {ev.finality && (
                                <Badge
                                  variant={ev.finality === "finalized" ? "neutral" : "warning"}
                                  className="mt-1 w-fit"
                                >
                                  {ev.finality}
                                </Badge>
                              )}
                            </div>
                          </TableCell>
                          <TableCell className="text-xs text-muted">
                            {fmtTime(ev.blockTimestamp)}
                          </TableCell>
                          <TableCell>
                            {link ? (
                              <a
                                className="font-mono text-xs text-primary hover:underline"
                                href={link}
                                target="_blank"
                                rel="noreferrer"
                                data-testid="trace-tx-link"
                              >
                                {ev.txHash ? shortAddress(ev.txHash) : "tx"} →
                              </a>
                            ) : (
                              <span className="text-muted">—</span>
                            )}
                          </TableCell>
                        </TableRow>
                      );
                    })}
                  </TableBody>
                </Table>
              </div>
            )}
          </>
        )}
      </CardContent>
    </Card>
  );
}
