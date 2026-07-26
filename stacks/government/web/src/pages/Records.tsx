import {
  Badge,
  Button,
  Card,
  Input,
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@dogtag/ui";
import { QrCode as QrIcon } from "lucide-react";
import { useEffect, useState } from "react";
import { Link } from "react-router-dom";
import { ShareQrDialog, type ShareQrTarget } from "../components/ShareQrDialog";
import { apiGet, apiPatch, apiPost, statusVariant, type GovRecord } from "../lib/api";

/** Is an ISO validUntil within 30 days of today (but not yet lapsed)? Drives the amber "expiring
 *  soon" chip — a lapsed window is already surfaced as the derived EXPIRED verdict. */
function expiringSoon(validUntil?: string | null): boolean {
  if (!validUntil) return false;
  const until = Date.parse(`${validUntil}T00:00:00Z`);
  if (Number.isNaN(until)) return false;
  const days = (until - Date.now()) / 86_400_000;
  return days >= 0 && days <= 30;
}

/** The authority's own credential ledger: every issued credential with its immutable on-chain proof
 *  (tx / block / contract / explorer link), the derived VALID/EXPIRED/REVOKED verdict, a link to the
 *  official receipt, editable off-chain metadata, and soft-invalidation. */
export function Records() {
  const [records, setRecords] = useState<GovRecord[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [editRoot, setEditRoot] = useState<string | null>(null);
  const [editLabel, setEditLabel] = useState("");
  const [editNotes, setEditNotes] = useState("");
  const [busy, setBusy] = useState<string | null>(null);
  const [shareTarget, setShareTarget] = useState<ShareQrTarget | null>(null);

  async function refresh() {
    setLoading(true);
    try {
      const json = await apiGet("/v1/records");
      setRecords((json.records as GovRecord[]) ?? []);
      setError(null);
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }
  useEffect(() => {
    void refresh();
  }, []);

  function openEdit(rec: GovRecord) {
    setEditRoot(rec.root);
    setEditLabel(rec.label ?? "");
    setEditNotes(rec.notes ?? "");
  }
  async function saveEdit() {
    if (!editRoot) return;
    setBusy(editRoot);
    try {
      const { status, json } = await apiPatch(`/v1/records/${editRoot}`, {
        label: editLabel || null,
        notes: editNotes || null,
      });
      if (status !== 200) setError(json.error || `HTTP ${status}`);
      else {
        setEditRoot(null);
        await refresh();
      }
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(null);
    }
  }
  async function revoke(rec: GovRecord) {
    if (!window.confirm("Revoke on-chain? It stays on record (as revoked) and remains verifiable.")) return;
    setBusy(rec.root);
    try {
      const { status, json } = await apiPost(
        `/v1/records/${rec.root}/revoke`,
        { reason: "credential withdrawn" },
        { auth: true },
      );
      if (status !== 200) setError(json.error || `HTTP ${status}`);
      else await refresh();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(null);
    }
  }
  async function markExpired(rec: GovRecord) {
    if (!window.confirm("Mark expired? It stays on record and keeps its on-chain proof.")) return;
    setBusy(rec.root);
    try {
      const { status, json } = await apiPatch(`/v1/records/${rec.root}`, {
        status: "expired",
        reason: "validity window lapsed",
      });
      if (status !== 200) setError(json.error || `HTTP ${status}`);
      else await refresh();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(null);
    }
  }

  return (
    <Card className="p-6">
      <div className="flex items-start justify-between gap-4">
        <div>
          <h2 className="text-base font-semibold text-onSurface">Issued credentials</h2>
          <p className="mt-1 max-w-2xl text-sm text-muted">
            Every credential this authority issued, from its own database — with the on-chain proof
            (tx, block, contract) and a block-explorer link. Revoked/expired credentials stay on
            record. Only off-chain metadata is editable; on-chain state is immutable. <strong>QR</strong>{" "}
            shows a one-time code the owner scans to import that credential onto their phone.
          </p>
        </div>
        <Button
          variant="outline"
          size="sm"
          data-testid="records-refresh"
          onClick={() => void refresh()}
          disabled={loading}
        >
          {loading ? "Loading…" : "Refresh"}
        </Button>
      </div>

      {error && (
        <p className="mt-4">
          <Badge variant="danger">{error}</Badge>
        </p>
      )}

      {records.length === 0 ? (
        <p className="mt-6 text-sm text-muted">
          {loading ? "Loading…" : "No credentials issued yet — issue your first travel clearance."}
        </p>
      ) : (
        <div className="mt-4">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>Type</TableHead>
                <TableHead>Dog tag</TableHead>
                <TableHead>Receipt</TableHead>
                <TableHead>Status</TableHead>
                <TableHead>On-chain proof</TableHead>
                <TableHead>Label</TableHead>
                <TableHead className="text-right">Actions</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {records.map((rec) => {
                const eff = rec.effectiveStatus || rec.status.toUpperCase();
                return (
                  <TableRow key={rec.root} data-testid="record-row" data-status={rec.status}>
                    <TableCell className="text-xs">{rec.recordType}</TableCell>
                    <TableCell className="mono font-mono text-xs">{rec.dogTagId}</TableCell>
                    <TableCell className="text-xs">
                      {rec.receiptId ? (
                        <Link
                          className="text-primary underline-offset-2 hover:underline"
                          to={`/receipt/${rec.root}`}
                        >
                          {rec.receiptId}
                        </Link>
                      ) : (
                        <span className="text-muted">—</span>
                      )}
                    </TableCell>
                    <TableCell>
                      <div className="flex flex-col items-start gap-1">
                        <Badge variant={statusVariant(eff)}>{eff}</Badge>
                        <span data-testid="record-status" className="text-[11px] text-muted">
                          {rec.status}
                        </span>
                        {rec.validUntil && (
                          <span className="text-[11px] tabular-nums text-muted">
                            until {rec.validUntil}
                          </span>
                        )}
                        {rec.validUntil && eff !== "REVOKED" && expiringSoon(rec.validUntil) && (
                          <Badge variant="warning">expires ≤30d</Badge>
                        )}
                      </div>
                    </TableCell>
                    <TableCell>
                      {rec.explorerUrl || rec.txHash ? (
                        <div className="text-xs">
                          <a
                            data-testid="explorer-link"
                            href={rec.explorerUrl || `https://explorer.roax.net/tx/${rec.txHash}`}
                            target="_blank"
                            rel="noreferrer"
                            className="font-mono text-primary hover:underline"
                          >
                            {rec.txHash?.slice(0, 10)}…
                          </a>
                          <div className="text-[11px] text-muted">block {rec.blockNumber ?? "?"}</div>
                          {rec.revokeExplorerUrl && (
                            <a
                              data-testid="revoke-explorer-link"
                              href={rec.revokeExplorerUrl}
                              target="_blank"
                              rel="noreferrer"
                              className="font-mono text-[11px] text-danger hover:underline"
                            >
                              revoke tx →
                            </a>
                          )}
                        </div>
                      ) : (
                        <span className="text-xs text-muted">—</span>
                      )}
                    </TableCell>
                    <TableCell className="text-xs text-muted">{rec.label || "—"}</TableCell>
                    <TableCell className="text-right">
                      <div className="flex justify-end gap-1 whitespace-nowrap">
                        <Button variant="ghost" size="sm" data-testid="edit-open" onClick={() => openEdit(rec)}>
                          Edit
                        </Button>
                        {/* Any listed credential can be handed to the owner's phone later — not just
                            the one issued in this session. Mirrors the vet's per-row QR action. */}
                        <Button
                          variant="outline"
                          size="sm"
                          data-testid="share-qr"
                          title="Show a one-time QR the owner scans to import this credential"
                          onClick={() =>
                            setShareTarget({
                              root: rec.root,
                              label: rec.receiptId
                                ? `receipt ${rec.receiptId}`
                                : `${rec.recordType} · tag ${rec.dogTagId}`,
                            })
                          }
                        >
                          <QrIcon className="h-4 w-4" /> QR
                        </Button>
                        {rec.status === "issued" && (
                          <Button
                            variant="ghost"
                            size="sm"
                            data-testid="expire"
                            disabled={busy === rec.root}
                            onClick={() => markExpired(rec)}
                          >
                            Expire
                          </Button>
                        )}
                        {(rec.status === "issued" || rec.status === "expired") && (
                          <Button
                            variant="ghost"
                            size="sm"
                            data-testid="revoke"
                            disabled={busy === rec.root}
                            onClick={() => revoke(rec)}
                          >
                            Revoke
                          </Button>
                        )}
                      </div>
                    </TableCell>
                  </TableRow>
                );
              })}
            </TableBody>
          </Table>
        </div>
      )}

      {editRoot && (
        <Card className="mt-6 border-accent/40 p-4" data-testid="edit-panel">
          <strong className="text-sm text-onSurface">Edit off-chain metadata</strong>
          <p className="mt-1 text-xs text-muted">
            Only the label and notes are editable. The tx hash, block, contract address and anchored
            document hash are immutable on-chain state.
          </p>
          <label className="mt-3 block text-xs font-medium text-muted">Label</label>
          <Input
            className="mt-1"
            data-testid="edit-label"
            value={editLabel}
            onChange={(e) => setEditLabel(e.target.value)}
          />
          <label className="mt-3 block text-xs font-medium text-muted">Notes</label>
          <Input
            className="mt-1"
            data-testid="edit-notes"
            value={editNotes}
            onChange={(e) => setEditNotes(e.target.value)}
          />
          <div className="mt-4 flex gap-2">
            <Button size="sm" data-testid="edit-save" disabled={busy === editRoot} onClick={() => void saveEdit()}>
              Save
            </Button>
            <Button size="sm" variant="outline" onClick={() => setEditRoot(null)}>
              Cancel
            </Button>
          </div>
        </Card>
      )}

      <ShareQrDialog target={shareTarget} onClose={() => setShareTarget(null)} />
    </Card>
  );
}
