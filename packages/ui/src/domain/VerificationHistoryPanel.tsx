import { CheckCircle2, Clock, History, RefreshCw, XCircle } from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import type { ApiClient } from "../api/client";
import type { VerificationHistoryItem } from "../api/types";
import { Badge } from "../components/Badge";
import { Button } from "../components/Button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "../components/Card";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "../components/Table";
import { explorerTxUrl } from "../wallet/chain";

export interface VerificationHistoryPanelProps {
  client: ApiClient;
  title?: string;
  description?: string;
}

function statusVariant(status: string): "success" | "warning" | "danger" | "neutral" {
  if (status === "recorded") return "success";
  if (status === "error") return "danger";
  if (status === "recording" || status === "pending") return "warning";
  return "neutral";
}

function StatusIcon({ status }: { status: string }) {
  if (status === "recorded") return <CheckCircle2 className="h-3.5 w-3.5" />;
  if (status === "error") return <XCircle className="h-3.5 w-3.5" />;
  return <Clock className="h-3.5 w-3.5" />;
}

function formatTime(seconds: number) {
  if (!seconds) return "Legacy";
  return new Date(seconds * 1000).toLocaleString();
}

function txHref(item: VerificationHistoryItem) {
  if (!item.txHash) return null;
  return item.explorerUrl || explorerTxUrl(item.txHash);
}

export function VerificationHistoryPanel({
  client,
  title = "Verification history",
  description = "Recent owner-consent verification sessions recorded by this verifier.",
}: VerificationHistoryPanelProps) {
  const [items, setItems] = useState<VerificationHistoryItem[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const resp = await client.verificationHistory();
      setItems(resp.verifications);
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setLoading(false);
    }
  }, [client]);

  useEffect(() => {
    void load();
  }, [load]);

  return (
    <Card>
      <CardHeader className="flex flex-col gap-3 sm:flex-row sm:items-start sm:justify-between">
        <div>
          <CardTitle className="flex items-center gap-2">
            <History className="h-5 w-5 text-primary" />
            {title}
          </CardTitle>
          <CardDescription>{description}</CardDescription>
        </div>
        <Button type="button" variant="outline" size="sm" onClick={() => void load()} loading={loading}>
          <RefreshCw className="h-4 w-4" /> Refresh
        </Button>
      </CardHeader>
      <CardContent>
        {error && <p className="mb-3 text-sm text-danger">{error}</p>}
        {items.length === 0 && !loading ? (
          <p className="rounded-md border border-dashed border-border p-4 text-sm text-muted">
            No verifications recorded yet.
          </p>
        ) : (
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>When</TableHead>
                <TableHead>Purpose</TableHead>
                <TableHead>Mode</TableHead>
                <TableHead>Status</TableHead>
                <TableHead>Proof</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {items.map((item) => {
                const href = txHref(item);
                return (
                  <TableRow key={item.sessionId}>
                    <TableCell className="whitespace-nowrap text-xs text-muted">
                      {formatTime(item.updatedAt || item.createdAt)}
                    </TableCell>
                    <TableCell>
                      <div className="min-w-0">
                        <p className="font-medium">{item.purpose}</p>
                        <p className="text-xs text-muted">{item.recordType}</p>
                      </div>
                    </TableCell>
                    <TableCell>
                      <Badge variant="neutral">{item.mode}</Badge>
                    </TableCell>
                    <TableCell>
                      <Badge variant={statusVariant(item.status)}>
                        <StatusIcon status={item.status} />
                        {item.status}
                      </Badge>
                    </TableCell>
                    <TableCell>
                      {href && item.txHash ? (
                        <a
                          href={href}
                          target="_blank"
                          rel="noreferrer"
                          className="block max-w-[16rem] truncate font-mono text-xs text-primary hover:underline"
                          title={item.txHash}
                        >
                          {item.txHash}
                        </a>
                      ) : (
                        <span className="text-xs text-muted">No tx yet</span>
                      )}
                      {item.nullifier && (
                        <p className="mt-1 max-w-[16rem] truncate font-mono text-xs text-muted" title={item.nullifier}>
                          nullifier {item.nullifier}
                        </p>
                      )}
                    </TableCell>
                  </TableRow>
                );
              })}
            </TableBody>
          </Table>
        )}
      </CardContent>
    </Card>
  );
}
