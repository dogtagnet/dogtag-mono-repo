import {
  Badge,
  Button,
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
  Input,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
  Spinner,
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
  explorerAddressUrl,
  explorerTxUrl,
  useToast,
  type ActivityEvent,
  type ActivityQuery,
  type ApiError,
} from "@dogtag/ui";
import { Activity as ActivityIcon, ExternalLink, Filter, RefreshCw, X } from "lucide-react";
import { useCallback, useEffect, useMemo, useState, type ReactNode } from "react";
import { useApp } from "../app/AppContext";
import { absoluteTime, eventMeta, EVENT_TYPES, relativeTime } from "../lib/activity";
import { shortAddr } from "../lib/format";

const PAGE_SIZE = 100;

/** Time-range presets → a `since` (unix seconds) lower bound, or `undefined` for "all time". */
const TIME_RANGES: { key: string; label: string; seconds: number | undefined }[] = [
  { key: "all", label: "All time", seconds: undefined },
  { key: "1h", label: "Last hour", seconds: 3600 },
  { key: "24h", label: "Last 24 hours", seconds: 86_400 },
  { key: "7d", label: "Last 7 days", seconds: 604_800 },
  { key: "30d", label: "Last 30 days", seconds: 2_592_000 },
];

const ALL = "__all__";

interface Filters {
  type: string;
  finality: string;
  range: string;
  signer: string;
  issuer: string;
  recordType: string;
}

const EMPTY: Filters = {
  type: ALL,
  finality: ALL,
  range: "all",
  signer: "",
  issuer: "",
  recordType: "",
};

/**
 * Activity - the UNSCOPED cross-issuer on-chain oversight feed (plan §3.2, the captain's "see on-chain
 * activity"). Renders the PR-B `/v1/admin/activity` proxy of the oversight indexer: every
 * IssuerCreated / RootRegistered / Whitelisted / Delisted / RootIssued / RootRevoked / Verified event,
 * newest-first, each with block/tx/timestamp, an explorer link, and the signer's BUSINESS NAME resolved
 * from the admin directory (`actorName`/`cloneName`). Filterable by issuer, signer, record type, event
 * type, finality, and time range. Non-PII throughout.
 */
export function Activity() {
  const { central } = useApp();
  const { toast } = useToast();
  const [draft, setDraft] = useState<Filters>(EMPTY);
  const [applied, setApplied] = useState<Filters>(EMPTY);
  const [events, setEvents] = useState<ActivityEvent[] | null>(null);
  const [total, setTotal] = useState(0);
  const [loading, setLoading] = useState(false);
  const [unavailable, setUnavailable] = useState<string | null>(null);
  const now = useMemo(() => Date.now(), [events]);

  const load = useCallback(
    async (f: Filters) => {
      setLoading(true);
      setUnavailable(null);
      const range = TIME_RANGES.find((r) => r.key === f.range);
      const q: ActivityQuery = {
        type: f.type === ALL ? undefined : (f.type as ActivityQuery["type"]),
        finality: f.finality === ALL ? undefined : (f.finality as ActivityQuery["finality"]),
        signer: f.signer || undefined,
        issuer: f.issuer || undefined,
        recordType: f.recordType || undefined,
        since: range?.seconds ? Math.floor(Date.now() / 1000) - range.seconds : undefined,
        limit: PAGE_SIZE,
      };
      try {
        const r = await central.getActivity(q);
        setEvents(r.events);
        setTotal(r.total);
      } catch (err) {
        const e = err as ApiError;
        // 503 = the oversight indexer isn't wired in this environment - a first-class empty state,
        // not an error toast (the surface is simply unavailable, everything else keeps working).
        if (e.status === 503) {
          setUnavailable(e.message);
          setEvents([]);
          setTotal(0);
        } else {
          toast({ title: "Failed to load activity", description: e.message, variant: "danger" });
          setEvents([]);
        }
      } finally {
        setLoading(false);
      }
    },
    [central, toast],
  );

  useEffect(() => {
    void load(applied);
  }, [load, applied]);

  const activeFilterCount = useMemo(() => {
    let n = 0;
    if (applied.type !== ALL) n++;
    if (applied.finality !== ALL) n++;
    if (applied.range !== "all") n++;
    if (applied.signer) n++;
    if (applied.issuer) n++;
    if (applied.recordType) n++;
    return n;
  }, [applied]);

  function apply() {
    setApplied(draft);
  }
  function reset() {
    setDraft(EMPTY);
    setApplied(EMPTY);
  }

  return (
    <div className="space-y-6">
      <Card>
        <CardHeader className="flex flex-row items-start justify-between gap-4">
          <div>
            <CardTitle className="flex items-center gap-2">
              <ActivityIcon className="h-5 w-5 text-primary" /> On-chain activity
            </CardTitle>
            <CardDescription>
              The unscoped, cross-issuer oversight feed - every issuance, revocation, whitelist change,
              and verification across all issuers, newest first. Signers are named from the business
              directory; every row links to the ROAX explorer. No client PII.
            </CardDescription>
          </div>
          <Button variant="outline" loading={loading} onClick={() => load(applied)}>
            <RefreshCw className="h-4 w-4" /> Refresh
          </Button>
        </CardHeader>
        <CardContent className="space-y-4">
          <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
            <FilterField label="Event type">
              <Select value={draft.type} onValueChange={(v) => setDraft((d) => ({ ...d, type: v }))}>
                <SelectTrigger>
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value={ALL}>All events</SelectItem>
                  {EVENT_TYPES.map((t) => (
                    <SelectItem key={t} value={t}>
                      {eventMeta(t).label}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </FilterField>
            <FilterField label="Finality">
              <Select
                value={draft.finality}
                onValueChange={(v) => setDraft((d) => ({ ...d, finality: v }))}
              >
                <SelectTrigger>
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value={ALL}>Any</SelectItem>
                  <SelectItem value="finalized">Finalized</SelectItem>
                  <SelectItem value="pending">Pending</SelectItem>
                </SelectContent>
              </Select>
            </FilterField>
            <FilterField label="Time range">
              <Select
                value={draft.range}
                onValueChange={(v) => setDraft((d) => ({ ...d, range: v }))}
              >
                <SelectTrigger>
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {TIME_RANGES.map((r) => (
                    <SelectItem key={r.key} value={r.key}>
                      {r.label}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </FilterField>
            <FilterField label="Signer address">
              <Input
                placeholder="0x… issuer signer"
                value={draft.signer}
                onChange={(e) => setDraft((d) => ({ ...d, signer: e.target.value }))}
              />
            </FilterField>
            <FilterField label="Issuer clone">
              <Input
                placeholder="0x… clone address"
                value={draft.issuer}
                onChange={(e) => setDraft((d) => ({ ...d, issuer: e.target.value }))}
              />
            </FilterField>
            <FilterField label="Record type">
              <Input
                placeholder="e.g. DOG_PROFILE"
                value={draft.recordType}
                onChange={(e) => setDraft((d) => ({ ...d, recordType: e.target.value }))}
              />
            </FilterField>
          </div>
          <div className="flex items-center gap-2">
            <Button onClick={apply} loading={loading}>
              <Filter className="h-4 w-4" /> Apply filters
            </Button>
            {activeFilterCount > 0 && (
              <Button variant="ghost" onClick={reset}>
                <X className="h-4 w-4" /> Clear ({activeFilterCount})
              </Button>
            )}
          </div>
        </CardContent>
      </Card>

      <Card>
        <CardContent className="pt-6">
          {events === null ? (
            <div className="flex justify-center py-12">
              <Spinner className="h-6 w-6 text-muted" />
            </div>
          ) : unavailable ? (
            <div className="rounded-md border border-dashed border-border bg-surface-muted p-6 text-center text-sm text-muted">
              <p className="font-medium text-onSurface">Oversight indexer not connected</p>
              <p className="mt-1">
                The activity feed reads from the oversight indexer, which is not configured in this
                environment. Set <code>INDEXER_API_BASE</code> on the central backend to enable it.
              </p>
            </div>
          ) : events.length === 0 ? (
            <p className="py-12 text-center text-sm text-muted">
              No on-chain events match these filters.
            </p>
          ) : (
            <>
              <div className="mb-3 flex items-center justify-between text-xs text-muted">
                <span>
                  Showing {events.length} of {total} event{total === 1 ? "" : "s"}
                </span>
                {total > events.length && <span>Narrow the filters to see older events.</span>}
              </div>
              <div className="overflow-x-auto">
                <Table>
                  <TableHeader>
                    <TableRow>
                      <TableHead>Event</TableHead>
                      <TableHead>Actor</TableHead>
                      <TableHead>Issuer clone</TableHead>
                      <TableHead>Record type</TableHead>
                      <TableHead>Block</TableHead>
                      <TableHead>When</TableHead>
                      <TableHead>Tx</TableHead>
                    </TableRow>
                  </TableHeader>
                  <TableBody>
                    {events.map((ev) => (
                      <EventRow key={ev.id} ev={ev} now={now} />
                    ))}
                  </TableBody>
                </Table>
              </div>
            </>
          )}
        </CardContent>
      </Card>
    </div>
  );
}

function FilterField({ label, children }: { label: string; children: ReactNode }) {
  return (
    <label className="block space-y-1">
      <span className="text-xs font-medium text-muted">{label}</span>
      {children}
    </label>
  );
}

/** One address rendered as a business name (when known) over a monospace explorer link. */
function ActorCell({ address, name }: { address?: string | null; name?: string | null }) {
  if (!address) return <span className="text-xs text-muted">-</span>;
  return (
    <div className="flex flex-col">
      {name && <span className="text-sm font-medium text-onSurface">{name}</span>}
      <a
        href={explorerAddressUrl(address)}
        target="_blank"
        rel="noreferrer"
        className="font-mono text-xs text-primary hover:underline"
      >
        {shortAddr(address)}
      </a>
    </div>
  );
}

function EventRow({ ev, now }: { ev: ActivityEvent; now: number }) {
  const meta = eventMeta(ev.type);
  const txUrl = ev.txUrl || (ev.txHash ? explorerTxUrl(ev.txHash) : undefined);
  return (
    <TableRow>
      <TableCell>
        <Badge variant={meta.variant}>{meta.label}</Badge>
      </TableCell>
      <TableCell>
        <ActorCell address={ev.actor} name={ev.actorName} />
      </TableCell>
      <TableCell>
        <ActorCell address={ev.clone} name={ev.cloneName} />
      </TableCell>
      <TableCell className="text-sm">
        {ev.recordType ? (
          <span className="font-medium">{ev.recordType}</span>
        ) : (
          <span className="text-xs text-muted">-</span>
        )}
      </TableCell>
      <TableCell className="font-mono text-xs">
        <div className="flex flex-col">
          <span>#{ev.blockNumber}</span>
          <Badge variant={ev.finality === "finalized" ? "neutral" : "warning"} className="mt-1 w-fit">
            {ev.finality}
          </Badge>
        </div>
      </TableCell>
      <TableCell className="text-xs" title={absoluteTime(ev.blockTimestamp)}>
        {relativeTime(ev.blockTimestamp, now)}
      </TableCell>
      <TableCell>
        {txUrl ? (
          <a
            href={txUrl}
            target="_blank"
            rel="noreferrer"
            className="inline-flex items-center gap-1 font-mono text-xs text-primary hover:underline"
          >
            {shortAddr(ev.txHash)} <ExternalLink className="h-3 w-3" />
          </a>
        ) : (
          <span className="text-xs text-muted">-</span>
        )}
      </TableCell>
    </TableRow>
  );
}
