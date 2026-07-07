import {
  Badge,
  Button,
  Card,
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@dogtag/ui";
import { useEffect, useState } from "react";
import {
  apiGetResult,
  eventMeta,
  type OversightActivityResp,
  type OversightEvent,
  type OversightStatsResp,
} from "../lib/api";

/** Short 0x… address/hash for dense table cells. */
function short(hex?: string): string {
  if (!hex) return "—";
  return hex.length > 12 ? `${hex.slice(0, 8)}…${hex.slice(-4)}` : hex;
}

function fmtTime(secs?: number): string {
  if (!secs) return "—";
  return new Date(secs * 1000).toLocaleString();
}

function Stat({ label, value }: { label: string; value: number | string }) {
  return (
    <div className="rounded-lg border border-border bg-surface-muted px-3 py-2">
      <div className="text-lg font-semibold text-onSurface">{value}</div>
      <div className="text-[11px] uppercase tracking-wide text-muted">{label}</div>
    </div>
  );
}

/**
 * The government oversight console (govarch PR-5): the UNSCOPED, cross-issuer on-chain credential feed
 * from the oversight indexer, joined to the authority's OWN issued credentials. Unlike the vet/groomer
 * traceability portal, this view sees EVERY issuer's activity - the government's own records are simply
 * highlighted. The feed is non-PII (roots are salted commitments; no importer/subject block).
 */
export function Oversight() {
  const [resp, setResp] = useState<OversightActivityResp | null>(null);
  const [stats, setStats] = useState<OversightStatsResp | null>(null);
  const [loading, setLoading] = useState(true);
  const [unavailable, setUnavailable] = useState(false);
  const [error, setError] = useState<string | null>(null);

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

  const events = resp?.events ?? [];

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

          {events.length === 0 ? (
            <p className="py-8 text-center text-sm text-muted" data-testid="oversight-empty">
              {loading ? "Loading…" : "No on-chain activity yet."}
            </p>
          ) : (
            <div className="mt-4 overflow-x-auto">
              <Table>
                <TableHeader>
                  <TableRow>
                    <TableHead>Event</TableHead>
                    <TableHead>Issuer</TableHead>
                    <TableHead>Record type</TableHead>
                    <TableHead>This authority</TableHead>
                    <TableHead>Block</TableHead>
                    <TableHead>When</TableHead>
                    <TableHead>Tx</TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {events.map((ev: OversightEvent) => {
                    const meta = eventMeta(ev.type);
                    return (
                      <TableRow
                        key={ev.id}
                        data-testid="oversight-event-row"
                        data-type={ev.type}
                        data-own={ev.local ? "true" : "false"}
                      >
                        <TableCell>
                          <Badge variant={meta.variant}>{meta.label}</Badge>
                        </TableCell>
                        <TableCell className="text-xs">
                          {ev.cloneName ? (
                            <span title={ev.clone}>{ev.cloneName}</span>
                          ) : (
                            <span className="font-mono text-muted">{short(ev.clone)}</span>
                          )}
                        </TableCell>
                        <TableCell className="text-xs">
                          {ev.recordType ?? <span className="text-muted">—</span>}
                        </TableCell>
                        <TableCell data-testid="oversight-local">
                          {ev.local ? (
                            <div className="flex flex-col gap-0.5">
                              <Badge variant="success" data-testid="oversight-local-own">
                                {ev.local.kind === "verification" ? "our verification" : "our record"}
                              </Badge>
                              <span className="font-mono text-[10px] text-muted">
                                {ev.local.receiptId ? ev.local.receiptId : short(ev.local.root)}
                                {ev.local.dogTagId ? ` · tag ${ev.local.dogTagId}` : ""}
                                {ev.local.status ? ` · ${ev.local.status}` : ""}
                              </span>
                            </div>
                          ) : (
                            <span className="text-[11px] text-muted" data-testid="oversight-local-other">
                              other issuer
                            </span>
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
                        <TableCell className="text-xs text-muted">{fmtTime(ev.blockTimestamp)}</TableCell>
                        <TableCell>
                          {ev.txUrl ? (
                            <a
                              className="font-mono text-xs text-primary hover:underline"
                              href={ev.txUrl}
                              target="_blank"
                              rel="noreferrer"
                              data-testid="oversight-tx-link"
                            >
                              {short(ev.txHash)} →
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
    </Card>
  );
}
