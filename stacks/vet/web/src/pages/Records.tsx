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
  DialogHeader,
  DialogTitle,
  QrCode,
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
  explorerTxUrl,
  useToast,
  type RecordStatus,
} from "@dogtag/ui";
import { QrCode as QrIcon, Ban } from "lucide-react";
import { useState } from "react";
import { useApp } from "../app/AppContext";
import { useRecordsStore, type LocalRecord } from "../app/recordsStore";

const statusVariant: Record<RecordStatus, "success" | "danger" | "warning"> = {
  issued: "success",
  revoked: "danger",
  prepared: "warning",
};

export function Records() {
  const { api } = useApp();
  const { toast } = useToast();
  const { records, setStatus } = useRecordsStore();
  const [qr, setQr] = useState<{ url: string; id: string } | null>(null);
  const [busyId, setBusyId] = useState<string | null>(null);

  async function regenQr(rec: LocalRecord) {
    setBusyId(rec.recordId);
    try {
      const r = await api.share(rec.recordId);
      setQr({ url: r.qrUrl, id: rec.recordId });
    } catch (err) {
      toast({ title: "Share failed", description: (err as Error).message, variant: "danger" });
    } finally {
      setBusyId(null);
    }
  }

  async function revoke(rec: LocalRecord) {
    if (!window.confirm("Revoke this credential on chain? This cannot be undone.")) return;
    setBusyId(rec.recordId);
    try {
      const r = await api.revoke(rec.recordId);
      setStatus(rec.recordId, "revoked", r.txHash);
      toast({ title: "Revoked", description: r.txHash, variant: "success" });
    } catch (err) {
      toast({ title: "Revoke failed", description: (err as Error).message, variant: "danger" });
    } finally {
      setBusyId(null);
    }
  }

  return (
    <Card>
      <CardHeader>
        <CardTitle>Records</CardTitle>
        <CardDescription>Credentials issued from this device. Re-generate a share QR or revoke.</CardDescription>
      </CardHeader>
      <CardContent>
        {records.length === 0 ? (
          <p className="py-8 text-center text-sm text-muted">No records issued yet.</p>
        ) : (
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>Record type</TableHead>
                <TableHead>Dog tag</TableHead>
                <TableHead>Status</TableHead>
                <TableHead>Tx</TableHead>
                <TableHead className="text-right">Actions</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {records.map((rec) => (
                <TableRow key={rec.recordId}>
                  <TableCell className="font-medium">{rec.recordType}</TableCell>
                  <TableCell className="font-mono text-xs">{rec.dogTagId}</TableCell>
                  <TableCell>
                    <Badge variant={statusVariant[rec.status]}>{rec.status}</Badge>
                  </TableCell>
                  <TableCell>
                    {rec.txHash ? (
                      <a
                        className="font-mono text-xs text-primary hover:underline"
                        href={explorerTxUrl(rec.txHash)}
                        target="_blank"
                        rel="noreferrer"
                      >
                        {rec.txHash.slice(0, 10)}…
                      </a>
                    ) : (
                      <span className="text-muted">—</span>
                    )}
                  </TableCell>
                  <TableCell className="text-right">
                    <div className="flex justify-end gap-2">
                      <Button
                        size="sm"
                        variant="outline"
                        loading={busyId === rec.recordId}
                        onClick={() => regenQr(rec)}
                      >
                        <QrIcon className="h-4 w-4" /> QR
                      </Button>
                      {rec.status === "issued" && (
                        <Button
                          size="sm"
                          variant="danger"
                          loading={busyId === rec.recordId}
                          onClick={() => revoke(rec)}
                        >
                          <Ban className="h-4 w-4" /> Revoke
                        </Button>
                      )}
                    </div>
                  </TableCell>
                </TableRow>
              ))}
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
    </Card>
  );
}
