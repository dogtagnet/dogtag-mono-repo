import {
  Badge,
  Button,
  Card,
  ChainTime,
  ChainValue,
  ProvenanceBadge,
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
} from "@dogtag/ui";
import { ChevronDown, ChevronRight } from "lucide-react";
import { Fragment, useEffect, useMemo, useState, type ReactNode } from "react";
import {
  apiGetResult,
  eventMeta,
  type OversightActivityResp,
  type OversightEvent,
  type OversightLocalJoin,
  type OversightStatsResp,
} from "../lib/api";

/**
 * The badge for an event joined to this authority's own data. An owner-blind `verified` event joins
 * via the field-hashed dogTagId only (`joinedBy: "dogTagId"`), which proves a TAG this authority
 * credentialed was verified - the event binds no root, so it never identifies WHICH credential;
 * `purpose` is the only disambiguator. Label at tag granularity, never "our credential was verified".
 */
function localBadgeLabel(local: OversightLocalJoin): string {
  if (local.joinedBy === "dogTagId") return "tag we credentialed";
  return local.kind === "verification" ? "our verification" : "our record";
}

function Stat({ label, value }: { label: string; value: number | string }) {
  return (
    <div className="rounded-lg border border-border bg-surface-muted px-3 py-2">
      <div className="text-lg font-semibold text-onSurface">{value}</div>
      <div className="text-[11px] uppercase tracking-wide text-muted">{label}</div>
    </div>
  );
}

/** One label/value line inside the expanded provenance panel. */
function PanelRow({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="flex flex-wrap items-baseline gap-x-2 gap-y-0.5">
      <span className="w-32 shrink-0 text-[11px] uppercase tracking-wide text-muted">{label}</span>
      <span className="min-w-0 text-xs text-onSurface">{children}</span>
    </div>
  );
}

/**
 * The full on-chain provenance of one event: everything the indexer knows, untruncated and copyable.
 * Every value here is rendered COMPLETE (wrapping rather than truncating) - this panel is the reason
 * the row above may safely truncate, so a truncated value here would leave the full form reachable
 * only by clipboard, which is precisely what the row already offers.
 *
 * Lives behind a row expander so the table itself stays scannable - the task of the row is to let an
 * auditor decide whether to look closer; this panel is the looking closer.
 */
function ProvenancePanel({ ev, columns }: { ev: OversightEvent; columns: number }) {
  const txHref = txExplorerHref(ev);
  const contractHref = addressExplorerHref(ev.contract);
  const cloneHref = addressExplorerHref(ev.clone);
  const actorHref = addressExplorerHref(ev.actor);
  const synthetic = chainProvenance(ev) !== "onchain";

  return (
    <TableRow data-testid="oversight-event-detail" data-event-id={ev.id}>
      <TableCell colSpan={columns} className="bg-surface-muted/60">
        <div className="grid gap-x-8 gap-y-1.5 py-1 md:grid-cols-2">
          <PanelRow label="Transaction">
            <TxRef event={ev} href={txHref} full testId="oversight-detail-tx" />
          </PanelRow>
          <PanelRow label="Emitted by">
            <ChainValue
              label={emittingContractRole(ev.type)}
              value={ev.contract}
              href={contractHref}
              full
              testId="oversight-detail-contract"
            />
          </PanelRow>
          <PanelRow label="Block">
            <span className="font-mono">
              #{ev.blockNumber ?? "?"}
              {typeof ev.logIndex === "number" ? ` · log ${ev.logIndex}` : ""}
            </span>
            {ev.blockHash && (
              <ChainValue
                label="block hash"
                value={ev.blockHash}
                className="ml-2"
                full
                testId="oversight-detail-blockhash"
              />
            )}
          </PanelRow>
          {/* The clone the event ACTED ON. For a factory-emitted event this is NOT the emitting
              contract above, which is why the name belongs here rather than under that address. */}
          <PanelRow label="Issuer clone">
            {ev.clone ? (
              <span className="inline-flex min-w-0 flex-wrap items-center gap-1">
                {ev.cloneName && <span className="text-onSurface">{ev.cloneName}</span>}
                <ChainValue label="at" value={ev.clone} href={cloneHref} full />
              </span>
            ) : (
              <span className="text-muted">— (not a clone-scoped event)</span>
            )}
          </PanelRow>
          <PanelRow label="On-chain time">
            <span>{formatChainTime(ev.onchainTs ?? ev.blockTimestamp)}</span>
            <span className="ml-2 font-mono text-[11px] text-muted">
              unix {ev.onchainTs ?? ev.blockTimestamp ?? "—"}
            </span>
          </PanelRow>
          <PanelRow label="Actor">
            {ev.actor ? (
              <span className="inline-flex min-w-0 flex-wrap items-center gap-1">
                {ev.actorName && <span className="text-onSurface">{ev.actorName}</span>}
                <ChainValue label="signer" value={ev.actor} href={actorHref} full />
              </span>
            ) : (
              <span className="text-muted">—</span>
            )}
          </PanelRow>
          {/* No join context here: the panel is the raw chain payload, so the field-hashed dogTagId
              is shown even when the row above names the same tag by its readable handle. */}
          {eventDetailFields(ev)
            .filter((f) => f.value)
            .map((f) => (
              <PanelRow key={f.label} label={f.label}>
                <ChainValue label={f.label} value={f.value} full labelHidden />
              </PanelRow>
            ))}
          {(ev.deadline ?? 0) > 0 && (
            <PanelRow label="Consent until">{formatChainTime(ev.deadline)}</PanelRow>
          )}
          {synthetic && (
            <div className="md:col-span-2">
              <p className="mt-1 text-xs text-warning" data-testid="oversight-synthetic-note">
                This event's transaction hash (<span className="font-mono">{ev.txHash ?? "none"}</span>)
                is not a well-formed 32-byte value, so it addresses no transaction on any chain and
                has no explorer page - the link is withheld rather than shown dead. The usual cause is
                a scripted/demo indexer feed.
              </p>
            </div>
          )}
        </div>
      </TableCell>
    </TableRow>
  );
}

const COLUMN_COUNT = 7;

/**
 * The government oversight console (govarch PR-5): the UNSCOPED, cross-issuer on-chain credential feed
 * from the oversight indexer, joined to the authority's OWN issued credentials. Unlike the vet/groomer
 * traceability portal, this view sees EVERY issuer's activity - the government's own records are simply
 * highlighted. The feed is non-PII (roots are salted commitments; no importer/subject block).
 *
 * As an AUDIT surface its obligation is that every claim be checkable, so each row carries the on-chain
 * time, the emitting contract, the block and the transaction - and states, per row, whether those
 * identifiers actually address a chain. See `@dogtag/ui` `chain/provenance` for the gating rule.
 */
export function Oversight() {
  const [resp, setResp] = useState<OversightActivityResp | null>(null);
  const [stats, setStats] = useState<OversightStatsResp | null>(null);
  const [loading, setLoading] = useState(true);
  const [unavailable, setUnavailable] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [expanded, setExpanded] = useState<Record<string, boolean>>({});

  async function refresh() {
    setLoading(true);
    setError(null);
    setUnavailable(false);
    try {
      const [a, s] = await Promise.all([
        apiGetResult("/v1/oversight/activity"),
        apiGetResult("/v1/oversight/stats"),
      ]);
      if (a.status === 503) {
        setUnavailable(true);
        setResp(null);
        setStats(null);
        return;
      }
      if (a.status !== 200) {
        const body = a.json as { error?: string } | null;
        throw new Error(body?.error || `HTTP ${a.status}`);
      }
      setResp(a.json as OversightActivityResp);
      setStats(s.status === 200 ? (s.json as OversightStatsResp) : null);
    } catch (e) {
      setError(String((e as Error).message ?? e));
    } finally {
      setLoading(false);
    }
  }

  useEffect(() => {
    void refresh();
  }, []);

  const events = useMemo(() => resp?.events ?? [], [resp]);
  const syntheticCount = useMemo(
    () => events.filter((ev) => chainProvenance(ev) !== "onchain").length,
    [events],
  );

  return (
    <Card className="p-6">
      <div className="flex flex-wrap items-start justify-between gap-2">
        <div>
          <h2 className="text-lg font-semibold text-onSurface">Oversight</h2>
          <p className="mt-1 max-w-2xl text-sm text-muted">
            Cross-issuer on-chain credential activity across ALL issuers (unscoped government
            oversight), joined to this authority's own issued credentials. Your own issuances are
            highlighted; every other issuer's activity is shown too. Non-PII by design.
          </p>
        </div>
        <Button
          size="sm"
          data-testid="oversight-refresh"
          disabled={loading}
          onClick={() => void refresh()}
        >
          {loading ? "Loading…" : "Refresh"}
        </Button>
      </div>

      {error && (
        <div className="mt-3">
          <Badge variant="danger">{error}</Badge>
        </div>
      )}

      {unavailable ? (
        <div
          className="mt-4 rounded-lg border border-dashed border-border bg-surface-muted px-4 py-8 text-center"
          data-testid="oversight-unavailable"
        >
          <p className="text-sm font-medium text-onSurface">Oversight indexer not connected</p>
          <p className="mx-auto mt-1 max-w-md text-xs text-muted">
            The oversight feed reads on-chain events from the indexer. It is not configured for this
            deployment yet (set <code>INDEXER_API_BASE</code> + the unscoped oversight token). Your
            issued credentials remain available on the Records page.
          </p>
        </div>
      ) : (
        <>
          <div className="mt-4 grid grid-cols-2 gap-2 sm:grid-cols-3 lg:grid-cols-6">
            <Stat label="Events" value={events.length} />
            <Stat label="Yours" value={resp?.matched ?? 0} />
            <Stat label="Issued" value={stats?.rootIssued ?? 0} />
            <Stat label="Revoked" value={stats?.rootRevoked ?? 0} />
            <Stat label="Verifications" value={stats?.verifications ?? 0} />
            <Stat label="Issuer clones" value={stats?.clones ?? 0} />
          </div>

          {/* Feed-level provenance. The topbar's LIVE CHAIN badge describes the ISSUANCE backend, not
              this feed - so without this line a fully scripted indexer reads as live chain history.
              State only what the data shows (a malformed hash) and name the demo feed as the LIKELY
              cause, not an asserted one: over-claiming is the failure this whole change exists to fix. */}
          {syntheticCount > 0 && (
            <div
              className="mt-4 rounded-lg border border-warning/40 bg-warning/10 px-3 py-2 text-xs text-warning"
              data-testid="oversight-synthetic-banner"
            >
              <span className="font-semibold">
                {syntheticCount} of {events.length}
              </span>{" "}
              event{syntheticCount === 1 ? "" : "s"} below carry a transaction hash that is not a
              well-formed 32-byte value, so it addresses no transaction on any chain — most often a
              scripted indexer feed (<code>INDEXER_DEMO_MODE</code>). Those rows are marked{" "}
              <span className="font-semibold">not chain-addressable</span> and their explorer links
              are withheld.
            </div>
          )}

          {events.length === 0 ? (
            <p className="py-8 text-center text-sm text-muted" data-testid="oversight-empty">
              {loading ? "Loading…" : "No on-chain activity yet."}
            </p>
          ) : (
            <div className="mt-4 overflow-x-auto">
              <Table>
                <TableHeader>
                  <TableRow>
                    {/* The four the captain has to be able to read WITHOUT scrolling - when, on
                        which contract, in which transaction, and whether that transaction is real -
                        lead the table. The joins scroll. */}
                    <TableHead>Event</TableHead>
                    <TableHead>When</TableHead>
                    <TableHead>On chain</TableHead>
                    <TableHead>Contract</TableHead>
                    <TableHead>Details</TableHead>
                    <TableHead>Actor</TableHead>
                    <TableHead>This authority</TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {events.map((ev: OversightEvent) => {
                    const meta = eventMeta(ev.type);
                    const provenance = chainProvenance(ev);
                    const isOpen = !!expanded[ev.id];
                    const contractHref = addressExplorerHref(ev.contract);
                    const actorHref = addressExplorerHref(ev.actor);
                    const emittedCloneName = emittingCloneName(ev);
                    const Chevron = isOpen ? ChevronDown : ChevronRight;
                    return (
                      <Fragment key={ev.id}>
                        <TableRow
                          data-testid="oversight-event-row"
                          data-type={ev.type}
                          data-own={ev.local ? "true" : "false"}
                          data-provenance={provenance}
                        >
                          <TableCell>
                            <button
                              type="button"
                              className="flex items-center gap-1 text-left"
                              aria-expanded={isOpen}
                              data-testid="oversight-expand"
                              onClick={() =>
                                setExpanded((prev) => ({ ...prev, [ev.id]: !prev[ev.id] }))
                              }
                            >
                              <Chevron className="h-3.5 w-3.5 shrink-0 text-muted" aria-hidden />
                              <Badge variant={meta.variant}>{meta.label}</Badge>
                            </button>
                          </TableCell>
                          <TableCell data-testid="oversight-when">
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
                              <TxRef
                                event={ev}
                                href={txExplorerHref(ev)}
                                testId="oversight-tx-link"
                              />
                              <ProvenanceBadge provenance={provenance} className="w-fit" />
                            </div>
                          </TableCell>
                          {/* WHICH smart contract emitted this. Not the same as the issuer clone for
                              factory/registry events, so the role is named alongside the address -
                              and the clone's NAME is shown only when the clone is what emitted, else
                              the factory would appear to be the named clone. */}
                          <TableCell className="text-xs" data-testid="oversight-contract">
                            <div className="flex max-w-[10rem] flex-col gap-0.5">
                              <ChainValue
                                label={emittingContractRole(ev.type)}
                                value={ev.contract}
                                href={contractHref}
                                head={8}
                                stacked
                                testId="oversight-contract-value"
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
                          <TableCell className="text-xs" data-testid="oversight-details">
                            <div className="flex max-w-[9rem] flex-col gap-0.5">
                              {/* A root/tx join names this very event, so its record type is shown.
                                  A tag-granular join does not: the join cell already names the SAME
                                  tag by its readable handle (so the field-hash would render one tag
                                  two ways), and WHICH credential was verified is unknowable. */}
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
                          <TableCell className="text-xs" data-testid="oversight-actor">
                            {/* The acting signer: issuing/revoking signer, or - on `verified` - the
                                relayer that submitted the consent proof. Central to oversight: WHO.
                                Named signers can be long, so they truncate to one line rather than
                                wrapping and stretching every row. */}
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
                          <TableCell data-testid="oversight-local">
                            {ev.local ? (
                              <div className="flex flex-col gap-0.5">
                                <Badge variant="success" data-testid="oversight-local-own">
                                  {localBadgeLabel(ev.local)}
                                </Badge>
                                {ev.local.receiptId ? (
                                  <span className="font-mono text-[10px] text-muted">
                                    receipt {ev.local.receiptId}
                                  </span>
                                ) : (
                                  <ChainValue
                                    label="root"
                                    value={ev.local.root}
                                    head={8}
                                    testId="oversight-local-root"
                                  />
                                )}
                                <span className="text-[10px] text-muted">
                                  {ev.local.dogTagId ? `tag ${ev.local.dogTagId}` : ""}
                                  {ev.local.status ? ` · ${ev.local.status}` : ""}
                                </span>
                              </div>
                            ) : (
                              <span
                                className="text-[11px] text-muted"
                                data-testid="oversight-local-other"
                              >
                                other issuer
                              </span>
                            )}
                          </TableCell>
                        </TableRow>
                        {isOpen && <ProvenancePanel ev={ev} columns={COLUMN_COUNT} />}
                      </Fragment>
                    );
                  })}
                </TableBody>
              </Table>
            </div>
          )}
        </>
      )}
    </Card>
  );
}
