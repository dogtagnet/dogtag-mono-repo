import {
  Badge,
  Button,
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
  Spinner,
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
  explorerAddressUrl,
  useToast,
  type IssuerApplicationListItem,
} from "@dogtag/ui";
import { CheckCircle2, HelpCircle, RefreshCw, ShieldCheck, XCircle } from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { useApp } from "../app/AppContext";
import { shortAddr } from "../lib/format";
import { isWhitelistedFor, whitelistConfigured } from "../lib/whitelist";

type OnChain = boolean | "error" | "unknown";

interface Entry {
  recordType: string;
  address: string;
  /** derived from the approved/delisted application status */
  derived: boolean;
  onChain: OnChain;
}

/**
 * Whitelist viewer (impl §5.3). Expands every (recordType, address) pair from the issuer
 * applications into a table. The DERIVED column reflects application status (approved → expected
 * whitelisted). When VITE_ISSUER_REGISTRY_ADDR is set, the ON-CHAIN column reads
 * IssuerRegistry.isWhitelistedFor(keccak256(recordType), address) directly via viem against ROAX.
 */
export function Whitelist() {
  const { central } = useApp();
  const { toast } = useToast();
  const [entries, setEntries] = useState<Entry[] | null>(null);
  const [reading, setReading] = useState(false);
  const configured = whitelistConfigured();

  const load = useCallback(async () => {
    try {
      const r = await central.listApplications();
      const rows = expand(r.applications);
      setEntries(rows);
    } catch (err) {
      toast({ title: "Failed to load applications", description: (err as Error).message, variant: "danger" });
      setEntries([]);
    }
  }, [central, toast]);

  useEffect(() => {
    void load();
  }, [load]);

  async function readOnChain() {
    if (!entries) return;
    setReading(true);
    try {
      const next = await Promise.all(
        entries.map(async (e): Promise<Entry> => {
          try {
            const ok = await isWhitelistedFor(e.recordType, e.address);
            return { ...e, onChain: ok };
          } catch {
            return { ...e, onChain: "error" };
          }
        }),
      );
      setEntries(next);
    } finally {
      setReading(false);
    }
  }

  return (
    <Card>
      <CardHeader className="flex flex-row items-start justify-between gap-4">
        <div>
          <CardTitle className="flex items-center gap-2">
            <ShieldCheck className="h-5 w-5 text-primary" /> Whitelist viewer
          </CardTitle>
          <CardDescription>
            On-chain issuer whitelist state per (recordType, address). Derived from application status;
            optionally read live from the IssuerRegistry.
          </CardDescription>
        </div>
        <Button variant="outline" loading={reading} disabled={!configured} onClick={readOnChain}>
          <RefreshCw className="h-4 w-4" /> Read on-chain
        </Button>
      </CardHeader>
      <CardContent>
        {!configured && (
          <p className="mb-4 rounded-md border border-dashed border-border bg-surface-muted p-3 text-xs text-muted">
            <code>VITE_ISSUER_REGISTRY_ADDR</code> is not set — the on-chain column is unavailable.
            The derived column below reflects issuer-application status (approved ⇒ expected
            whitelisted). Set the registry address to enable live{" "}
            <code>isWhitelistedFor</code> reads via viem.
          </p>
        )}
        {entries === null ? (
          <div className="flex justify-center py-8">
            <Spinner className="h-5 w-5 text-muted" />
          </div>
        ) : entries.length === 0 ? (
          <p className="py-8 text-center text-sm text-muted">No (recordType, address) entries.</p>
        ) : (
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>Record type</TableHead>
                <TableHead>Address</TableHead>
                <TableHead>Derived</TableHead>
                <TableHead>On-chain</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {entries.map((e) => (
                <TableRow key={`${e.recordType}:${e.address}`}>
                  <TableCell className="font-medium">{e.recordType}</TableCell>
                  <TableCell>
                    <a
                      href={explorerAddressUrl(e.address)}
                      target="_blank"
                      rel="noreferrer"
                      className="font-mono text-xs text-primary hover:underline"
                    >
                      {shortAddr(e.address)}
                    </a>
                  </TableCell>
                  <TableCell>
                    <Badge variant={e.derived ? "success" : "neutral"}>
                      {e.derived ? <CheckCircle2 className="h-3 w-3" /> : <XCircle className="h-3 w-3" />}
                      {e.derived ? "whitelisted" : "not whitelisted"}
                    </Badge>
                  </TableCell>
                  <TableCell>
                    <OnChainBadge state={e.onChain} />
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        )}
      </CardContent>
    </Card>
  );
}

function OnChainBadge({ state }: { state: OnChain }) {
  if (state === "unknown") return <span className="text-xs text-muted">—</span>;
  if (state === "error")
    return (
      <Badge variant="warning">
        <HelpCircle className="h-3 w-3" /> read error
      </Badge>
    );
  return (
    <Badge variant={state ? "success" : "neutral"}>
      {state ? <CheckCircle2 className="h-3 w-3" /> : <XCircle className="h-3 w-3" />}
      {state ? "whitelisted" : "not whitelisted"}
    </Badge>
  );
}

/** Expand applications into a deduped (recordType, address) entry table. */
function expand(apps: IssuerApplicationListItem[]): Entry[] {
  const map = new Map<string, Entry>();
  for (const a of apps) {
    const derived = a.status === "approved";
    for (const address of a.addresses) {
      for (const recordType of a.recordTypes) {
        const key = `${recordType}:${address.toLowerCase()}`;
        const existing = map.get(key);
        // approved wins over not-approved for the derived flag.
        if (!existing || (derived && !existing.derived)) {
          map.set(key, { recordType, address, derived, onChain: "unknown" });
        }
      }
    }
  }
  return [...map.values()].sort((x, y) => x.recordType.localeCompare(y.recordType));
}
