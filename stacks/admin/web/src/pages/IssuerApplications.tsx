import {
  Badge,
  Button,
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
  Spinner,
  explorerTxUrl,
  useToast,
  type IssuerApplicationListItem,
  type IssuerApplicationStatus,
} from "@dogtag/ui";
import { Check, ListChecks, Slash, X } from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { useApp } from "../app/AppContext";
import { shortAddr } from "../lib/format";

const statusVariant: Record<IssuerApplicationStatus, "warning" | "success" | "danger" | "neutral"> = {
  pending: "warning",
  approved: "success",
  rejected: "danger",
  delisted: "neutral",
};

/**
 * Issuer applications queue (impl §5.3 / §4.3). Lists applications from GET /v1/issuer-applications.
 * Approve triggers the central backend's on-chain `whitelistFor` for EACH (address, recordType)
 * pair (after DNS-TXT + accreditation checks); reject and delist are also wired. Shows the
 * multi-address-per-entity / multi-recordType structure.
 */
export function IssuerApplications() {
  const { central } = useApp();
  const { toast } = useToast();
  const [apps, setApps] = useState<IssuerApplicationListItem[] | null>(null);
  const [busyId, setBusyId] = useState<string | null>(null);
  const [txs, setTxs] = useState<Record<string, string[]>>({});

  const load = useCallback(async () => {
    try {
      const r = await central.listApplications();
      setApps(r.applications);
    } catch (err) {
      toast({ title: "Failed to load applications", description: (err as Error).message, variant: "danger" });
      setApps([]);
    }
  }, [central, toast]);

  useEffect(() => {
    void load();
  }, [load]);

  async function approve(id: string) {
    setBusyId(id);
    try {
      const r = await central.approveApplication(id);
      setTxs((p) => ({ ...p, [id]: r.whitelistTxs }));
      toast({
        title: "Approved",
        description: `${r.whitelistTxs.length} whitelistFor tx(s) sent`,
        variant: "success",
      });
      await load();
    } catch (err) {
      toast({ title: "Approve failed", description: (err as Error).message, variant: "danger" });
    } finally {
      setBusyId(null);
    }
  }

  async function reject(id: string) {
    setBusyId(id);
    try {
      await central.rejectApplication(id);
      toast({ title: "Rejected", variant: "success" });
      await load();
    } catch (err) {
      toast({ title: "Reject failed", description: (err as Error).message, variant: "danger" });
    } finally {
      setBusyId(null);
    }
  }

  async function delist(id: string) {
    if (!window.confirm("Delist this issuer? This sends an on-chain delistFor per (address,recordType).")) return;
    setBusyId(id);
    try {
      const r = await central.delistApplication(id);
      setTxs((p) => ({ ...p, [id]: r.delistTxs }));
      toast({ title: "Delisted", description: `${r.delistTxs.length} delistFor tx(s) sent`, variant: "success" });
      await load();
    } catch (err) {
      toast({ title: "Delist failed", description: (err as Error).message, variant: "danger" });
    } finally {
      setBusyId(null);
    }
  }

  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex items-center gap-2">
          <ListChecks className="h-5 w-5 text-primary" /> Issuer applications
        </CardTitle>
        <CardDescription>
          Approve triggers on-chain <code>whitelistFor(keccak256(recordType), address)</code> for every
          (address × recordType) pair, after DNS-TXT + accreditation checks.
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-4">
        {apps === null ? (
          <div className="flex justify-center py-8">
            <Spinner className="h-5 w-5 text-muted" />
          </div>
        ) : apps.length === 0 ? (
          <p className="py-8 text-center text-sm text-muted">No applications.</p>
        ) : (
          apps.map((a) => (
            <div key={a.applicationId} className="space-y-3 rounded-lg border border-border p-4">
              <div className="flex flex-wrap items-center justify-between gap-2">
                <div>
                  <div className="font-medium text-onSurface">{a.issuerEntityId}</div>
                  <div className="text-xs text-muted">{a.domain}</div>
                </div>
                <Badge variant={statusVariant[a.status]}>{a.status}</Badge>
              </div>

              <div className="grid gap-3 sm:grid-cols-2">
                <div>
                  <div className="mb-1 text-xs font-medium text-muted">Addresses ({a.addresses.length})</div>
                  <div className="flex flex-wrap gap-1.5">
                    {a.addresses.map((addr) => (
                      <Badge key={addr} variant="neutral">
                        <span className="font-mono">{shortAddr(addr)}</span>
                      </Badge>
                    ))}
                  </div>
                </div>
                <div>
                  <div className="mb-1 text-xs font-medium text-muted">Record types ({a.recordTypes.length})</div>
                  <div className="flex flex-wrap gap-1.5">
                    {a.recordTypes.map((rt) => (
                      <Badge key={rt} variant="default">
                        {rt}
                      </Badge>
                    ))}
                  </div>
                </div>
              </div>

              <p className="text-xs text-muted">
                {a.addresses.length} × {a.recordTypes.length} ={" "}
                {a.addresses.length * a.recordTypes.length} on-chain whitelist entries.
              </p>

              {txs[a.applicationId]?.length ? (
                <div className="flex flex-wrap gap-2">
                  {txs[a.applicationId]!.map((tx) => (
                    <a
                      key={tx}
                      href={explorerTxUrl(tx)}
                      target="_blank"
                      rel="noreferrer"
                      className="font-mono text-xs text-primary hover:underline"
                    >
                      {tx.slice(0, 10)}…
                    </a>
                  ))}
                </div>
              ) : null}

              <div className="flex flex-wrap gap-2">
                {a.status === "pending" && (
                  <>
                    <Button size="sm" loading={busyId === a.applicationId} onClick={() => approve(a.applicationId)}>
                      <Check className="h-4 w-4" /> Approve
                    </Button>
                    <Button
                      size="sm"
                      variant="outline"
                      loading={busyId === a.applicationId}
                      onClick={() => reject(a.applicationId)}
                    >
                      <X className="h-4 w-4" /> Reject
                    </Button>
                  </>
                )}
                {a.status === "approved" && (
                  <Button
                    size="sm"
                    variant="danger"
                    loading={busyId === a.applicationId}
                    onClick={() => delist(a.applicationId)}
                  >
                    <Slash className="h-4 w-4" /> Delist
                  </Button>
                )}
              </div>
            </div>
          ))
        )}
      </CardContent>
    </Card>
  );
}
