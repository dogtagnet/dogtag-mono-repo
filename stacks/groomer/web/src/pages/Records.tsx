import {
  Badge,
  Button,
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  Input,
  Label,
  QrCode,
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
  explorerTxUrl,
  useToast,
  type DbRecord,
  type RecordStatus,
} from "@dogtag/ui";
import { QrCode as QrIcon, Ban, Clock, Pencil, RefreshCw } from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { useApp } from "../app/AppContext";

const statusVariant: Record<RecordStatus, "success" | "danger" | "warning"> = {
  issued: "success",
  revoked: "danger",
  expired: "warning",
  prepared: "warning",
  confirming: "warning",
};

/** The explorer link for a record: the persisted `explorer_url`, or one derived from its tx hash. */
function txLink(rec: DbRecord): string | null {
  if (rec.explorer_url) return rec.explorer_url;
  if (rec.tx_hash) return explorerTxUrl(rec.tx_hash);
  return null;
}

export function Records() {
  const { api } = useApp();
  const { toast } = useToast();
  const [records, setRecords] = useState<DbRecord[]>([]);
  const [loading, setLoading] = useState(true);
  const [qr, setQr] = useState<{ url: string; id: string } | null>(null);
  const [edit, setEdit] = useState<DbRecord | null>(null);
  const [editLabel, setEditLabel] = useState("");
  const [editNotes, setEditNotes] = useState("");
  const [busyId, setBusyId] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    setLoading(true);
    try {
      const r = await api.listRecords();
      setRecords(r.records ?? []);
    } catch (err) {
      toast({ title: "Load failed", description: (err as Error).message, variant: "danger" });
    } finally {
      setLoading(false);
    }
  }, [api, toast]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  async function regenQr(rec: DbRecord) {
    setBusyId(rec.record_id);
    try {
      const r = await api.share(rec.record_id);
      setQr({ url: r.qrUrl, id: rec.record_id });
    } catch (err) {
      toast({ title: "Share failed", description: (err as Error).message, variant: "danger" });
    } finally {
      setBusyId(null);
    }
  }

  async function revoke(rec: DbRecord) {
    if (!window.confirm("Revoke this credential on chain? It stays on record (as revoked) and remains verifiable.")) return;
    setBusyId(rec.record_id);
    try {
      const r = await api.revoke(rec.record_id);
      toast({ title: "Revoked on-chain", description: r.txHash, variant: "success" });
      await refresh();
    } catch (err) {
      toast({ title: "Revoke failed", description: (err as Error).message, variant: "danger" });
    } finally {
      setBusyId(null);
    }
  }

  async function markExpired(rec: DbRecord) {
    if (!window.confirm("Mark this credential expired? It stays on record and keeps its on-chain proof.")) return;
    setBusyId(rec.record_id);
    try {
      await api.updateRecord(rec.record_id, { status: "expired", reason: "validity window lapsed" });
      toast({ title: "Marked expired", variant: "success" });
      await refresh();
    } catch (err) {
      toast({ title: "Update failed", description: (err as Error).message, variant: "danger" });
    } finally {
      setBusyId(null);
    }
  }

  function openEdit(rec: DbRecord) {
    setEdit(rec);
    setEditLabel(rec.label ?? "");
    setEditNotes(rec.notes ?? "");
  }

  async function saveEdit() {
    if (!edit) return;
    setBusyId(edit.record_id);
    try {
      await api.updateRecord(edit.record_id, { label: editLabel || null, notes: editNotes || null });
      toast({ title: "Saved", description: "Off-chain metadata updated.", variant: "success" });
      setEdit(null);
      await refresh();
    } catch (err) {
      toast({ title: "Save failed", description: (err as Error).message, variant: "danger" });
    } finally {
      setBusyId(null);
    }
  }

  return (
    <Card>
      <CardHeader>
        <div className="flex items-start justify-between gap-2">
          <div>
            <CardTitle>Records</CardTitle>
            <CardDescription>
              Every credential this platform issued, from its own database - with the on-chain proof
              (tx, block, contract) and a block-explorer link to verify it. Revoked/expired records stay
              on record.
            </CardDescription>
          </div>
          <Button
            size="sm"
            variant="outline"
            data-testid="records-refresh"
            onClick={() => void refresh()}
            loading={loading}
          >
            <RefreshCw className="h-4 w-4" /> Refresh
          </Button>
        </div>
      </CardHeader>
      <CardContent>
        {records.length === 0 ? (
          <p className="py-8 text-center text-sm text-muted">
            {loading ? "Loading…" : "No records issued yet."}
          </p>
        ) : (
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>Record type</TableHead>
                <TableHead>Dog tag</TableHead>
                <TableHead>Status</TableHead>
                <TableHead>On-chain proof</TableHead>
                <TableHead>Label</TableHead>
                <TableHead className="text-right">Actions</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {records.map((rec) => {
                const link = txLink(rec);
                return (
                  <TableRow key={rec.record_id} data-testid="record-row" data-status={rec.status}>
                    <TableCell className="font-medium">{rec.record_type}</TableCell>
                    <TableCell className="font-mono text-xs">{rec.dog_tag_id}</TableCell>
                    <TableCell>
                      <Badge variant={statusVariant[rec.status] ?? "warning"} data-testid="record-status">
                        {rec.status}
                      </Badge>
                    </TableCell>
                    <TableCell>
                      {link ? (
                        <div className="flex flex-col gap-0.5">
                          <a
                            className="font-mono text-xs text-primary hover:underline"
                            href={link}
                            target="_blank"
                            rel="noreferrer"
                            data-testid="explorer-link"
                          >
                            {rec.tx_hash?.slice(0, 12)}…
                          </a>
                          <span className="text-[10px] text-muted">
                            block {rec.block_number ?? "?"} · {rec.issuer_addr.slice(0, 8)}…
                          </span>
                          {rec.revoke_explorer_url && (
                            <a
                              className="font-mono text-[10px] text-danger hover:underline"
                              href={rec.revoke_explorer_url}
                              target="_blank"
                              rel="noreferrer"
                              data-testid="revoke-explorer-link"
                            >
                              revoke tx →
                            </a>
                          )}
                        </div>
                      ) : (
                        <span className="text-muted">—</span>
                      )}
                    </TableCell>
                    <TableCell className="max-w-[12rem] truncate text-xs text-muted">
                      {rec.label || "—"}
                    </TableCell>
                    <TableCell className="text-right">
                      <div className="flex justify-end gap-2">
                        <Button
                          size="sm"
                          variant="ghost"
                          data-testid="edit-open"
                          onClick={() => openEdit(rec)}
                          title="Edit metadata"
                        >
                          <Pencil className="h-4 w-4" />
                        </Button>
                        <Button
                          size="sm"
                          variant="outline"
                          data-testid="share-qr"
                          loading={busyId === rec.record_id}
                          onClick={() => regenQr(rec)}
                        >
                          <QrIcon className="h-4 w-4" /> QR
                        </Button>
                        {rec.status === "issued" && (
                          <Button
                            size="sm"
                            variant="ghost"
                            data-testid="expire"
                            loading={busyId === rec.record_id}
                            onClick={() => markExpired(rec)}
                            title="Mark expired (off-chain)"
                          >
                            <Clock className="h-4 w-4" />
                          </Button>
                        )}
                        {(rec.status === "issued" || rec.status === "expired") && (
                          <Button
                            size="sm"
                            variant="danger"
                            data-testid="revoke"
                            loading={busyId === rec.record_id}
                            onClick={() => revoke(rec)}
                          >
                            <Ban className="h-4 w-4" /> Revoke
                          </Button>
                        )}
                      </div>
                    </TableCell>
                  </TableRow>
                );
              })}
            </TableBody>
          </Table>
        )}
      </CardContent>

      <Dialog open={qr !== null} onOpenChange={(o) => !o && setQr(null)}>
        <DialogContent className="flex flex-col items-center">
          <DialogHeader>
            <DialogTitle>Share QR</DialogTitle>
            <DialogDescription>One-time, expires in ~3 minutes. The owner scans to import.</DialogDescription>
          </DialogHeader>
          {qr && <QrCode value={qr.url} caption={qr.url} />}
        </DialogContent>
      </Dialog>

      <Dialog open={edit !== null} onOpenChange={(o) => !o && setEdit(null)}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Edit record metadata</DialogTitle>
            <DialogDescription>
              Only off-chain metadata is editable. The tx hash, block, contract address and anchored
              document hash are immutable on-chain state.
            </DialogDescription>
          </DialogHeader>
          <div className="flex flex-col gap-3">
            <div className="flex flex-col gap-1">
              <Label htmlFor="rec-label">Label</Label>
              <Input
                id="rec-label"
                data-testid="edit-label"
                value={editLabel}
                onChange={(e) => setEditLabel(e.target.value)}
                placeholder="e.g. Rex — annual booster"
              />
            </div>
            <div className="flex flex-col gap-1">
              <Label htmlFor="rec-notes">Notes</Label>
              <Input
                id="rec-notes"
                data-testid="edit-notes"
                value={editNotes}
                onChange={(e) => setEditNotes(e.target.value)}
                placeholder="internal note"
              />
            </div>
          </div>
          <DialogFooter>
            <Button variant="outline" onClick={() => setEdit(null)}>
              Cancel
            </Button>
            <Button data-testid="edit-save" loading={busyId === edit?.record_id} onClick={() => void saveEdit()}>
              Save
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </Card>
  );
}
