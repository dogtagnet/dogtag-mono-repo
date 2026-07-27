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
  ProvenanceBadge,
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
  TxRef,
  addressExplorerHref,
  chainProvenance,
  emittingCloneName,
  emittingContractRole,
  eventDetailFields,
  formatChainTime,
  joinedDetailContext,
  shortHex,
  txExplorerHref,
  useToast,
  type ApiError,
  type TraceActivityResp,
  type TraceEventType,
  type TraceStatsResp,
} from "@dogtag/ui";
import { RefreshCw, ShieldCheck } from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
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

/** Human label for the joined local row's kind. */
function localKindLabel(kind?: string): string {
  if (kind === "verification") return "verification";
  if (kind === "mint") return "dog tag";
  return "record";
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

  const events = useMemo(() => resp?.events ?? [], [resp]);
  const scopeLabel = resp?.scope?.label;
  const syntheticCount = useMemo(
    () => events.filter((ev) => chainProvenance(ev) !== "onchain").length,
    [events],
  );

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

            {/* Feed-level provenance. An event whose tx hash is not a well-formed 32-byte value came
                from a scripted/demo indexer feed, not from chain history - and must not read as if it
                did. See `@dogtag/ui` `chain/provenance`. */}
            {syntheticCount > 0 && (
              <div
                className="mb-4 rounded-lg border border-warning/40 bg-warning/10 px-3 py-2 text-xs text-warning"
                data-testid="trace-synthetic-banner"
              >
                <span className="font-semibold">
                  {syntheticCount} of {events.length}
                </span>{" "}
                event{syntheticCount === 1 ? "" : "s"} below carry a transaction hash that is not a
                well-formed 32-byte value, so it addresses no transaction on any chain — most often a
                scripted indexer feed. Those rows are marked{" "}
                <span className="font-semibold">not chain-addressable</span> and their explorer links
                are withheld.
              </div>
            )}

            {events.length === 0 ? (
              <p className="py-8 text-center text-sm text-muted" data-testid="trace-empty">
                {loading ? "Loading…" : "No on-chain activity in scope yet."}
              </p>
            ) : (
              <div className="overflow-x-auto">
                <Table>
                  <TableHeader>
                    <TableRow>
                      {/* Time, transaction+provenance and contract lead; the local joins scroll. */}
                      <TableHead>Event</TableHead>
                      <TableHead>When</TableHead>
                      <TableHead>On chain</TableHead>
                      <TableHead>Contract</TableHead>
                      <TableHead>Details</TableHead>
                      <TableHead>Your record</TableHead>
                      <TableHead>Signer</TableHead>
                    </TableRow>
                  </TableHeader>
                  <TableBody>
                    {events.map((ev) => {
                      const meta = eventMeta(ev.type);
                      const provenance = chainProvenance(ev);
                      const contractHref = addressExplorerHref(ev.contract);
                      const actorHref = addressExplorerHref(ev.actor);
                      const emittedCloneName = emittingCloneName(ev);
                      return (
                        <TableRow
                          key={ev.id}
                          data-testid="trace-event-row"
                          data-type={ev.type}
                          data-provenance={provenance}
                        >
                          <TableCell>
                            <Badge variant={meta.variant}>{meta.label}</Badge>
                          </TableCell>
                          <TableCell data-testid="trace-when">
                            <ChainTime seconds={ev.blockTimestamp} />
                          </TableCell>
                          {/* Block + finality + transaction + the per-row provenance verdict. */}
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
                              <TxRef event={ev} href={txExplorerHref(ev)} testId="trace-tx-link" />
                              <ProvenanceBadge provenance={provenance} className="w-fit" />
                            </div>
                          </TableCell>
                          {/* WHICH smart contract emitted this. Not always the issuer clone -
                              `issuerCreated`/`rootRegistered` come from the factory, `verified` from
                              the verification registry - so the role is named beside the address, and
                              the clone's NAME appears only when the clone is what emitted. */}
                          <TableCell className="text-xs" data-testid="trace-contract">
                            <div className="flex max-w-[10rem] flex-col gap-0.5">
                              <ChainValue
                                label={emittingContractRole(ev.type)}
                                value={ev.contract}
                                href={contractHref}
                                head={8}
                                stacked
                                testId="trace-contract-value"
                              />
                              {emittedCloneName && (
                                <span
                                  className="truncate text-[10px] text-muted"
                                  title={emittedCloneName}
                                >
                                  {emittedCloneName}
                                </span>
                              )}
                            </div>
                          </TableCell>
                          {/* Every identifier is labelled: truncated 32-byte hex is otherwise
                              indistinguishable between a root, a record-type key and a nullifier.
                              The joined session carries the READABLE purpose and record type, which
                              the owner-blind chain payload does not - prefer them over the keys. */}
                          <TableCell className="text-xs" data-testid="trace-details">
                            <div className="flex max-w-[9rem] flex-col gap-0.5">
                              {eventDetailFields(ev, joinedDetailContext(ev.local))
                                .filter((f) => f.value)
                                .map((f) => (
                                  <ChainValue
                                    key={f.label}
                                    label={f.label}
                                    value={f.value}
                                    head={8}
                                    stacked
                                  />
                                ))}
                              {(ev.deadline ?? 0) > 0 && (
                                <span className="text-[10px] text-muted">
                                  consent until {formatChainTime(ev.deadline)}
                                </span>
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
                              actorHref ? (
                                <a
                                  className="block max-w-[7rem] truncate font-mono text-primary hover:underline"
                                  href={actorHref}
                                  target="_blank"
                                  rel="noreferrer"
                                  title={ev.actorName ? `${ev.actorName} · ${ev.actor}` : ev.actor}
                                >
                                  {ev.actorName ?? shortHex(ev.actor, 8)}
                                </a>
                              ) : (
                                <span
                                  className="block max-w-[7rem] truncate font-mono text-muted"
                                  title={ev.actor}
                                >
                                  {ev.actorName ?? shortHex(ev.actor, 8)}
                                </span>
                              )
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
