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
  Spinner,
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
  explorerAddressUrl,
  explorerTxUrl,
  useToast,
  type GovernanceDisposition,
  type IssuerApplicationListItem,
  type WhitelistGrantResp,
} from "@dogtag/ui";
import {
  CheckCircle2,
  HelpCircle,
  Plus,
  RefreshCw,
  Send,
  ShieldCheck,
  ShieldOff,
  XCircle,
} from "lucide-react";
import { useCallback, useEffect, useState, type FormEvent } from "react";
import { useApp } from "../app/AppContext";
import { shortAddr } from "../lib/format";
import { isWhitelistedFor, whitelistConfigured } from "../lib/whitelist";

type OnChain = boolean | "error" | "unknown";
type ActionKind = "grant" | "revoke";

interface Entry {
  recordType: string;
  address: string;
  /** derived from the approved/delisted application status */
  derived: boolean;
  onChain: OnChain;
}

/** The dispositions returned by a grant/revoke, kept per row for an inline audit/result trail. */
interface RowResult {
  action: ActionKind;
  actions: GovernanceDisposition[];
  issuerRole?: WhitelistGrantResp["issuerRole"];
}

const rowKey = (recordType: string, address: string) => `${recordType}:${address.toLowerCase()}`;
const isDogProfile = (recordType: string) => recordType.toUpperCase() === "DOG_PROFILE";

/**
 * Whitelist MANAGEMENT console (plan PR-E, §3.4). Promotes the read-only viewer to a control plane:
 * each (recordType, address) row can be granted (`whitelistFor` + verify keys + DOG_PROFILE
 * ISSUER_ROLE) or revoked (`delistFor`) on demand, and an ad-hoc "Grant capability" dialog whitelists
 * an arbitrary (signer, recordType, verifyPurposes) pair. Every write is routed through the backend's
 * `GovernanceAction` abstraction: it EXECUTES (broadcast tx + explorer link) while the hosted key
 * holds WHITELIST_ADMIN, and returns a PROPOSAL payload the moment that role moves to the governance
 * signer (Phase-2) — the console never assumes the old EOA. The derived + live on-chain columns keep
 * the original state view intact.
 */
export function Whitelist() {
  const { central } = useApp();
  const { toast } = useToast();
  const [entries, setEntries] = useState<Entry[] | null>(null);
  const [reading, setReading] = useState(false);
  const [results, setResults] = useState<Record<string, RowResult>>({});
  const [pending, setPending] = useState<{ action: ActionKind; recordType: string; address: string } | null>(null);
  const [dispatching, setDispatching] = useState(false);
  const [grantOpen, setGrantOpen] = useState(false);
  const configured = whitelistConfigured();

  const load = useCallback(async () => {
    try {
      const r = await central.listApplications();
      setEntries(expand(r.applications));
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
            return { ...e, onChain: await isWhitelistedFor(e.recordType, e.address) };
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

  /** Re-read a single row's on-chain state after a grant/revoke (best-effort). */
  async function refreshRow(recordType: string, address: string) {
    if (!configured) return;
    try {
      const ok = await isWhitelistedFor(recordType, address);
      setEntries((prev) =>
        prev
          ? prev.map((e) =>
              e.recordType === recordType && e.address.toLowerCase() === address.toLowerCase()
                ? { ...e, onChain: ok }
                : e,
            )
          : prev,
      );
    } catch {
      /* leave the prior on-chain value untouched */
    }
  }

  async function confirmPending() {
    if (!pending) return;
    const { action, recordType, address } = pending;
    const key = rowKey(recordType, address);
    setDispatching(true);
    try {
      const resp =
        action === "grant"
          ? await central.whitelistGrant({ signer: address, recordType })
          : await central.whitelistRevoke({ signer: address, recordType });
      const issuerRole = action === "grant" ? (resp as WhitelistGrantResp).issuerRole : undefined;
      setResults((p) => ({ ...p, [key]: { action, actions: resp.actions, issuerRole } }));
      toast({
        title: action === "grant" ? "Grant dispatched" : "Revoke dispatched",
        ...dispatchOutcome(resp, issuerRole),
      });
      setPending(null);
      void refreshRow(recordType, address);
    } catch (err) {
      toast({ title: `${action === "grant" ? "Grant" : "Revoke"} failed`, description: (err as Error).message, variant: "danger" });
    } finally {
      setDispatching(false);
    }
  }

  return (
    <Card>
      <CardHeader className="flex flex-row items-start justify-between gap-4">
        <div>
          <CardTitle className="flex items-center gap-2">
            <ShieldCheck className="h-5 w-5 text-primary" /> Whitelist management
          </CardTitle>
          <CardDescription>
            Grant or revoke issuer/verifier capabilities per (recordType, address), routed through the{" "}
            <code>GovernanceAction</code> layer — executes directly while the hosted key holds{" "}
            <code>WHITELIST_ADMIN</code>, else returns a proposal for the governance signer. Derived +
            live on-chain state below.
          </CardDescription>
        </div>
        <div className="flex flex-wrap items-center gap-2">
          <Button onClick={() => setGrantOpen(true)}>
            <Plus className="h-4 w-4" /> Grant capability
          </Button>
          <Button variant="outline" loading={reading} disabled={!configured} onClick={readOnChain}>
            <RefreshCw className="h-4 w-4" /> Read on-chain
          </Button>
        </div>
      </CardHeader>
      <CardContent>
        {!configured && (
          <p className="mb-4 rounded-md border border-dashed border-border bg-surface-muted p-3 text-xs text-muted">
            <code>VITE_ISSUER_REGISTRY_ADDR</code> is not set — the live on-chain column is unavailable.
            Grant/revoke still dispatch via the backend; set the registry address to confirm state with
            live <code>isWhitelistedFor</code> reads.
          </p>
        )}
        {entries === null ? (
          <div className="flex justify-center py-8">
            <Spinner className="h-5 w-5 text-muted" />
          </div>
        ) : entries.length === 0 ? (
          <p className="py-8 text-center text-sm text-muted">
            No (recordType, address) entries. Use <strong>Grant capability</strong> to whitelist an
            address directly.
          </p>
        ) : (
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>Record type</TableHead>
                <TableHead>Address</TableHead>
                <TableHead>Derived</TableHead>
                <TableHead>On-chain</TableHead>
                <TableHead className="text-right">Manage</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {entries.map((e) => {
                const key = rowKey(e.recordType, e.address);
                const result = results[key];
                return (
                  <TableRow key={key}>
                    <TableCell className="font-medium">
                      {e.recordType}
                      {isDogProfile(e.recordType) && (
                        <span className="ml-1.5 text-[10px] uppercase tracking-wide text-muted">
                          + ISSUER
                        </span>
                      )}
                    </TableCell>
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
                    <TableCell>
                      <div className="flex justify-end gap-2">
                        <Button
                          size="sm"
                          variant="outline"
                          onClick={() => setPending({ action: "grant", recordType: e.recordType, address: e.address })}
                        >
                          <ShieldCheck className="h-3.5 w-3.5" /> Grant
                        </Button>
                        <Button
                          size="sm"
                          variant="outline"
                          onClick={() => setPending({ action: "revoke", recordType: e.recordType, address: e.address })}
                        >
                          <ShieldOff className="h-3.5 w-3.5" /> Revoke
                        </Button>
                      </div>
                      {result && (
                        <div className="mt-2">
                          <DispositionList result={result} />
                        </div>
                      )}
                    </TableCell>
                  </TableRow>
                );
              })}
            </TableBody>
          </Table>
        )}
      </CardContent>

      <ConfirmActionDialog
        pending={pending}
        busy={dispatching}
        onCancel={() => (dispatching ? undefined : setPending(null))}
        onConfirm={confirmPending}
      />
      <GrantCapabilityDialog open={grantOpen} onOpenChange={setGrantOpen} />
    </Card>
  );
}

/** Confirm dialog for a per-row grant/revoke. */
function ConfirmActionDialog({
  pending,
  busy,
  onCancel,
  onConfirm,
}: {
  pending: { action: ActionKind; recordType: string; address: string } | null;
  busy: boolean;
  onCancel: () => void;
  onConfirm: () => void;
}) {
  const grant = pending?.action === "grant";
  return (
    <Dialog open={pending !== null} onOpenChange={(o) => (!o ? onCancel() : undefined)}>
      <DialogContent className="max-w-md">
        <DialogHeader>
          <DialogTitle>{grant ? "Grant capability" : "Revoke capability"}</DialogTitle>
          <DialogDescription>
            {grant ? "Whitelist" : "Delist"} <code>{pending?.recordType}</code> for{" "}
            <span className="font-mono text-xs">{pending && shortAddr(pending.address)}</span> on the
            IssuerRegistry.
          </DialogDescription>
        </DialogHeader>
        <div className="rounded-md border border-border bg-surface-muted p-3 text-xs text-muted">
          Routed through <code>GovernanceAction</code>: broadcasts <code>{grant ? "whitelistFor" : "delistFor"}</code>{" "}
          if the hosted key holds <code>WHITELIST_ADMIN</code>, otherwise returns a proposal payload for
          the governance signer to execute out-of-band.
          {grant && pending && isDogProfile(pending.recordType) && (
            <span className="mt-2 block">
              This is a <strong>DOG_PROFILE</strong> grant — it also grants{" "}
              <code>DogTagSBT.ISSUER_ROLE</code> (mint rights).
            </span>
          )}
        </div>
        <DialogFooter>
          <Button variant="outline" disabled={busy} onClick={onCancel}>
            Cancel
          </Button>
          <Button variant={grant ? "primary" : "danger"} loading={busy} onClick={onConfirm}>
            <Send className="h-4 w-4" /> {grant ? "Grant" : "Revoke"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

/** Ad-hoc "grant an arbitrary (signer, recordType, verifyPurposes) capability" dialog (plan §3.4). */
function GrantCapabilityDialog({ open, onOpenChange }: { open: boolean; onOpenChange: (o: boolean) => void }) {
  const { central } = useApp();
  const { toast } = useToast();
  const [signer, setSigner] = useState("");
  const [recordType, setRecordType] = useState("");
  const [verifyPurposes, setVerifyPurposes] = useState("");
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<RowResult | null>(null);

  function reset() {
    setSigner("");
    setRecordType("");
    setVerifyPurposes("");
    setResult(null);
  }

  async function submit(e: FormEvent) {
    e.preventDefault();
    const purposes = verifyPurposes.split(",").map((s) => s.trim()).filter(Boolean);
    if (!recordType.trim() && purposes.length === 0) {
      toast({ title: "Nothing to grant", description: "Provide a record type and/or verify purposes.", variant: "danger" });
      return;
    }
    setBusy(true);
    try {
      const resp = await central.whitelistGrant({
        signer: signer.trim(),
        recordType: recordType.trim() || undefined,
        verifyPurposes: purposes.length ? purposes : undefined,
      });
      setResult({ action: "grant", actions: resp.actions, issuerRole: resp.issuerRole });
      toast({ title: "Grant dispatched", ...dispatchOutcome(resp, resp.issuerRole) });
    } catch (err) {
      toast({ title: "Grant failed", description: (err as Error).message, variant: "danger" });
    } finally {
      setBusy(false);
    }
  }

  return (
    <Dialog
      open={open}
      onOpenChange={(o) => {
        onOpenChange(o);
        if (!o) reset();
      }}
    >
      <DialogContent className="max-w-lg">
        <DialogHeader>
          <DialogTitle>Grant capability</DialogTitle>
          <DialogDescription>
            Whitelist a signer for a record type and/or VERIFY:&lt;purpose&gt; capabilities directly —
            decoupled from the application queue (key rotation, ad-hoc grants, incident response). A{" "}
            <code>DOG_PROFILE</code> record type also grants <code>ISSUER_ROLE</code>.
          </DialogDescription>
        </DialogHeader>
        <form onSubmit={submit} className="grid gap-3">
          <div className="space-y-1.5">
            <Label required>Signer address</Label>
            <Input value={signer} onChange={(ev) => setSigner(ev.target.value)} placeholder="0x…" required />
          </div>
          <div className="space-y-1.5">
            <Label>Record type</Label>
            <Input
              value={recordType}
              onChange={(ev) => setRecordType(ev.target.value)}
              placeholder="VACCINATION (or 0x-prefixed key)"
            />
          </div>
          <div className="space-y-1.5">
            <Label>Verify purposes (comma)</Label>
            <Input
              value={verifyPurposes}
              onChange={(ev) => setVerifyPurposes(ev.target.value)}
              placeholder="grooming_intake, boarding_intake"
            />
          </div>
          {result && <DispositionList result={result} />}
          <DialogFooter>
            <Button type="submit" loading={busy}>
              <Send className="h-4 w-4" /> Grant
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}

/** Render the dispositions from a grant/revoke: executed → tx explorer link; proposed → payload. */
function DispositionList({ result }: { result: RowResult }) {
  const { actions, issuerRole } = result;
  return (
    <div className="space-y-1.5 rounded-md border border-border bg-surface-muted p-2 text-xs">
      {actions.map((d, i) => (
        <DispositionRow key={i} label={result.action === "grant" ? "whitelistFor" : "delistFor"} d={d} />
      ))}
      {issuerRole &&
        ("disposition" in issuerRole ? (
          <DispositionRow label="grantRole(ISSUER)" d={issuerRole} />
        ) : (
          <div className="flex items-center gap-1.5 text-muted">
            <CheckCircle2 className="h-3 w-3 text-success" /> ISSUER_ROLE already held
          </div>
        ))}
    </div>
  );
}

function DispositionRow({ label, d }: { label: string; d: GovernanceDisposition }) {
  if (d.disposition === "executed") {
    return (
      <div className="flex flex-wrap items-center gap-1.5">
        <Badge variant="success">
          <CheckCircle2 className="h-3 w-3" /> {label}
        </Badge>
        <a
          href={explorerTxUrl(d.txHash)}
          target="_blank"
          rel="noreferrer"
          className="font-mono text-primary hover:underline"
        >
          {d.txHash.slice(0, 12)}…
        </a>
      </div>
    );
  }
  return (
    <div className="flex flex-wrap items-center gap-1.5">
      <Badge variant="warning">
        <Send className="h-3 w-3" /> {label} · proposed
      </Badge>
      <span className="text-muted">
        holder{" "}
        <span className="font-mono">{d.holder ? shortAddr(d.holder) : "unknown"}</span>
      </span>
      <span className="font-mono text-muted" title={d.calldata}>
        {d.calldata.slice(0, 12)}…
      </span>
    </div>
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
        const key = rowKey(recordType, address);
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

/** One-line summary of a dispatch: how many capabilities executed vs proposed.
 *
 *  When NOTHING executed, say so plainly. The old wording always attributed a proposal to
 *  "(governance)", which reads as the intended Phase-2 handover even when the real cause is that this
 *  stack booted a key holding no authority at all - in that case nothing lands on-chain and the row
 *  still looked like a success. */
function summarize(actions: GovernanceDisposition[]): string {
  const executed = actions.filter((a) => a.disposition === "executed").length;
  const proposed = actions.length - executed;
  if (!actions.length) return "no capabilities";
  if (!executed) {
    return `${proposed} proposed - NOTHING broadcast, on-chain state unchanged`;
  }
  const parts: string[] = [`${executed} executed`];
  if (proposed) parts.push(`${proposed} proposed (awaiting the authority holder)`);
  return parts.join(" · ");
}

/** `issuerRole` is a real dispatched action for a DOG_PROFILE grant, but it can also be the
 *  non-disposition `{status:"alreadyHeld"}` or null. Only an actual disposition counts toward what
 *  reached the chain. */
function issuerRoleDisposition(
  issuerRole: WhitelistGrantResp["issuerRole"],
): GovernanceDisposition | null {
  return issuerRole && "disposition" in issuerRole ? issuerRole : null;
}

/** How a dispatch is REPORTED. A grant/revoke where nothing reached the chain must never render green:
 *  a success toast around "NOTHING broadcast" is exactly the indistinguishable-from-success signal the
 *  loud-authority work removes. The backend's `executed`/`warning` are authoritative (they account for
 *  the separately dispatched ISSUER_ROLE action); `executed` is optional, so an older backend falls back
 *  to recomputing over every disposition - including issuerRole, which `summarize` alone cannot see. */
function dispatchOutcome(
  resp: { actions: GovernanceDisposition[]; executed?: boolean; warning?: string | null },
  issuerRole?: WhitelistGrantResp["issuerRole"],
): { variant: "success" | "danger"; description: string } {
  const role = issuerRoleDisposition(issuerRole);
  const all = role ? [...resp.actions, role] : resp.actions;
  const executed = resp.executed ?? all.some((a) => a.disposition === "executed");
  return {
    variant: executed ? "success" : "danger",
    description: resp.warning ?? summarize(all),
  };
}
