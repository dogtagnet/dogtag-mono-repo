import {
  Badge,
  Button,
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
  ChainTime,
  ChainValue,
  Input,
  ProvenanceBadge,
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
  TxRef,
  addressChainRef,
  chainProvenance,
  emittingCloneName,
  emittingContractRole,
  eventDetailFields,
  formatChainTime,
  txExplorerHref,
  useToast,
  type ActivityEvent,
  type ActivityQuery,
  type ApiError,
} from "@dogtag/ui";
import { Activity as ActivityIcon, Filter, RefreshCw, X } from "lucide-react";
import { useCallback, useEffect, useMemo, useState, type ReactNode } from "react";
import { useApp } from "../app/AppContext";
import { eventMeta, EVENT_TYPES } from "../lib/activity";
import { AddressRef } from "../components/ChainRef";

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
 * newest-first, each with the on-chain time, the emitting contract, the block, the transaction, and the
 * signer's BUSINESS NAME resolved from the admin directory (`actorName`/`cloneName`). Filterable by
 * issuer, signer, record type, event type, finality, and time range. Non-PII throughout.
 *
 * This is an AUDIT surface, so it uses the shared `@dogtag/ui` `chain/` primitives that the government
 * Oversight and vet Traceability tables were migrated onto in PR #88 - one implementation, so the three
 * consoles cannot drift into separate dialects of "here is the transaction". The rule those primitives
 * carry, and the reason this page was in scope at all: a row may claim an explorer link ONLY when its
 * transaction hash is a well-formed 32-byte value. This table previously linked EVERY row to
 * `explorer.roax.net`, so a scripted indexer feed's `0x0800` rendered as a working on-chain reference
 * to a transaction that cannot exist. See `chain/provenance.ts` for why the test is arithmetic rather
 * than a demo-mode flag: it also catches ONE synthetic row inside an otherwise-real feed.
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
  const syntheticCount = useMemo(
    () => (events ?? []).filter((ev) => chainProvenance(ev) !== "onchain").length,
    [events],
  );

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
            {/* "every row links to the ROAX explorer" used to be stated here, and it was the defect
                written down as a promise: the link was emitted whether or not the transaction could
                exist. State the condition instead of the blanket claim. */}
            <CardDescription>
              The unscoped, cross-issuer oversight feed - every issuance, revocation, whitelist change,
              and verification across all issuers, newest first. Signers are named from the business
              directory. A row links to the ROAX explorer only when its transaction hash is a
              well-formed 32-byte value; rows that carry anything else are marked{" "}
              <span className="font-medium">not chain-addressable</span> and their links are withheld.
              No client PII.
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

              {/* Feed-level provenance, as government Oversight and vet Traceability both carry. The
                  per-row badge is what makes a single bad row visible; this line is what makes a
                  WHOLLY scripted feed visible without reading every row. It states only what the data
                  shows - a malformed hash - and names the demo feed as the likely cause, not an
                  asserted one. */}
              {syntheticCount > 0 && (
                <div
                  className="mb-3 rounded-lg border border-warning/40 bg-warning/10 px-3 py-2 text-xs text-warning"
                  data-testid="activity-synthetic-banner"
                >
                  <span className="font-semibold">
                    {syntheticCount} of {events.length}
                  </span>{" "}
                  events below carr{syntheticCount === 1 ? "ies" : "y"} a transaction hash that is not
                  a well-formed 32-byte value, so it addresses no transaction on any chain - most
                  often a scripted indexer feed (<code>INDEXER_DEMO_MODE</code>). Th
                  {syntheticCount === 1 ? "at row is" : "ose rows are"} marked{" "}
                  <span className="font-semibold">not chain-addressable</span> and{" "}
                  {syntheticCount === 1 ? "its" : "their"} explorer link
                  {syntheticCount === 1 ? " is" : "s are"} withheld.
                </div>
              )}

              <div className="overflow-x-auto">
                <Table>
                  <TableHeader>
                    <TableRow>
                      {/* What an auditor must read WITHOUT scrolling leads the table: when, whether
                          the transaction is real, and which contract emitted it. The directory joins
                          (actor, clone) scroll. `Record type` folded into `Details`, where it is
                          labelled alongside the event's other identifiers - the filter input above is
                          a query param and is unaffected. */}
                      <TableHead>Event</TableHead>
                      <TableHead>When</TableHead>
                      <TableHead>On chain</TableHead>
                      <TableHead>Contract</TableHead>
                      <TableHead>Details</TableHead>
                      <TableHead>Actor</TableHead>
                      <TableHead>Issuer clone</TableHead>
                    </TableRow>
                  </TableHeader>
                  <TableBody>
                    {events.map((ev) => (
                      <EventRow key={ev.id} ev={ev} />
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

/**
 * One address rendered as a business name (when known) over its monospace address.
 *
 * The address links to the explorer only when it is a well-formed 20-byte EVM address. This is the
 * same rule the transaction reference follows and it is here for the same reason: this cell used to
 * call `explorerAddressUrl` unconditionally, so a malformed `actor`/`clone` on the very feed that can
 * carry a synthetic row rendered a real-looking address link that leads nowhere. An unlinkable address
 * is still shown - the value is what the indexer reported and hiding it would lose an auditor a fact -
 * it simply is not dressed as something that resolves.
 *
 * That rule now lives in the shared `AddressRef`, which every other admin page was migrated onto: this
 * cell was the FIRST to get it right and became, for a while, the only one, so keeping a second copy
 * here would leave the portal with two implementations of one rule and no way to notice them drifting.
 * The name is suppressed along with a missing address, since a business name floating above a bare
 * dash reads as a row about that business rather than a row with nothing to identify.
 */
function ActorCell({ address, name }: { address?: string | null; name?: string | null }) {
  return (
    <div className="flex flex-col">
      {name && address && <span className="text-sm font-medium text-onSurface">{name}</span>}
      <AddressRef address={address} testId="activity-actor" />
    </div>
  );
}

function EventRow({ ev }: { ev: ActivityEvent }) {
  const meta = eventMeta(ev.type);
  const provenance = chainProvenance(ev);
  const contract = addressChainRef(ev.contract);
  const emittedCloneName = emittingCloneName(ev);
  return (
    <TableRow
      data-testid="activity-event-row"
      data-type={ev.type}
      data-provenance={provenance}
    >
      <TableCell>
        <Badge variant={meta.variant}>{meta.label}</Badge>
      </TableCell>
      {/* Absolute time WITH a timezone, plus a relative hint. "3 min ago" alone is not something an
          auditor can cite, and the tooltip that used to carry the absolute form survives neither a
          screenshot nor a touch device. */}
      <TableCell data-testid="activity-when">
        <ChainTime seconds={ev.blockTimestamp} />
      </TableCell>
      {/* Block + finality + transaction + the per-row provenance verdict, together: the block and the
          transaction are one claim about where this event sits on chain, and the verdict says whether
          that claim can be checked. `TxRef` renders the hash INERT rather than linked when it cannot
          address a transaction - a struck-through value, never a dead anchor. */}
      <TableCell className="text-xs">
        <div className="flex flex-col gap-1">
          <span className="whitespace-nowrap font-mono">
            #{ev.blockNumber ?? "?"}
            {ev.finality && (
              <Badge
                variant={ev.finality === "finalized" ? "neutral" : "warning"}
                className="ml-1.5"
              >
                {ev.finality}
              </Badge>
            )}
          </span>
          <TxRef event={ev} href={txExplorerHref(ev)} testId="activity-tx-link" />
          <ProvenanceBadge provenance={provenance} className="w-fit" />
        </div>
      </TableCell>
      {/* WHICH smart contract emitted this - a question the table could not answer before, and one an
          unscoped cross-issuer feed needs most. `contract` is not always the issuer clone:
          `issuerCreated`/`rootRegistered` come from the factory, `whitelisted`/`delisted` from the
          issuer registry, `verified` from the verification registry. The clone's NAME appears here
          only when the clone is what emitted, else the factory would read as the named clone; the
          clone keeps its own column. */}
      <TableCell className="text-xs" data-testid="activity-contract">
        <div className="flex max-w-[10rem] flex-col gap-0.5">
          <ChainValue
            label={emittingContractRole(ev.type)}
            value={ev.contract}
            href={contract.href}
            reason={contract.reason}
            head={8}
            stacked
            testId="activity-contract-value"
          />
          {emittedCloneName && (
            <span className="truncate text-[10px] text-muted" title={emittedCloneName}>
              {emittedCloneName}
            </span>
          )}
        </div>
      </TableCell>
      {/* Every identifier labelled and copyable. Truncated 32-byte hex is otherwise indistinguishable
          between a credential root, a hashed record-type key, a purpose key and a nullifier - and this
          feed is unscoped, so there is no local join to fall back on: these identifiers are all the
          reader gets. `eventDetailFields` follows the SHAPE of `recordType`, which arrives either as a
          human label or as its keccak key depending on whether the indexer could reverse it. */}
      <TableCell className="text-xs" data-testid="activity-details">
        <div className="flex max-w-[10rem] flex-col gap-0.5">
          {eventDetailFields(ev)
            .filter((f) => f.value)
            .map((f) => (
              <ChainValue key={f.label} label={f.label} value={f.value} head={8} stacked />
            ))}
          {(ev.deadline ?? 0) > 0 && (
            <span className="text-[10px] text-muted">
              consent until {formatChainTime(ev.deadline)}
            </span>
          )}
        </div>
      </TableCell>
      <TableCell data-testid="activity-actor">
        <ActorCell address={ev.actor} name={ev.actorName} />
      </TableCell>
      <TableCell data-testid="activity-clone">
        <ActorCell address={ev.clone} name={ev.cloneName} />
      </TableCell>
    </TableRow>
  );
}
