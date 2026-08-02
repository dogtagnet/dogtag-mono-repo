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
  Input,
  Label,
  Spinner,
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
  CopyButton,
  addressChainRef,
  txChainRef,
  useToast,
  type CreateIssuerResp,
  type GovernanceAuthorityResp,
  type IssuerCloneStat,
  demoIssuerDeploy,
} from "@dogtag/ui";
import { CheckCircle2, Copy, Factory, Plus, Send, ShieldCheck,
  Sparkles,
} from "lucide-react";
import { useCallback, useEffect, useRef, useState, type FormEvent } from "react";

import { env } from "../lib/env";
import { useApp } from "../app/AppContext";
import { AddressRef } from "../components/ChainRef";

/**
 * Issuers / Factory (plan §3.3, PR-C). Deploy new `DogTagIssuer` clones from the
 * `DogTagIssuerFactory` with a LIVE deterministic clone-address preview (`predictIssuer`, salt =
 * keccak256(recordType, business)) shown BEFORE the operator commits. The deploy runs through the
 * backend's key-holder-agnostic `GovernanceAction` layer (PR-A): if the hosted operator key IS the
 * factory `Ownable` owner it signs + broadcasts `createIssuer`; once ownership has moved to the
 * governance signer (Phase-2) it returns the calldata as a proposal instead. The authority banner
 * surfaces which path a deploy will take so there are no surprises. The clone list is best-effort:
 * it needs the oversight indexer and degrades gracefully when that is unwired.
 */
export function Issuers() {
  const { central } = useApp();
  const { toast } = useToast();
  const [authority, setAuthority] = useState<GovernanceAuthorityResp | null>(null);
  const [clones, setClones] = useState<IssuerCloneStat[] | null>(null);
  const [clonesError, setClonesError] = useState<string | null>(null);
  const [open, setOpen] = useState(false);
  const [result, setResult] = useState<CreateIssuerResp | null>(null);

  const loadAuthority = useCallback(async () => {
    try {
      setAuthority(await central.governanceAuthority());
    } catch (err) {
      toast({ title: "Failed to read authority map", description: (err as Error).message, variant: "danger" });
    }
  }, [central, toast]);

  const loadClones = useCallback(async () => {
    try {
      const r = await central.listIssuers();
      setClones(r.issuers);
      setClonesError(null);
    } catch (err) {
      // The clone list needs the oversight indexer; a 503/unwired indexer must not break the page.
      setClones([]);
      setClonesError((err as Error).message);
    }
  }, [central]);

  useEffect(() => {
    void loadAuthority();
    void loadClones();
  }, [loadAuthority, loadClones]);

  return (
    <div className="space-y-6">
      <AuthorityBanner authority={authority} />

      <Card>
        <CardHeader className="flex flex-row items-start justify-between gap-4">
          <div>
            <CardTitle className="flex items-center gap-2">
              <Factory className="h-5 w-5 text-primary" /> Issuers / Factory
            </CardTitle>
            <CardDescription>
              Deployed <code>DogTagIssuer</code> clones and their cross-issuer issuance activity.
              Deploy a new clone from the factory with a live deterministic address preview.
            </CardDescription>
          </div>
          <Button onClick={() => setOpen(true)}>
            <Plus className="h-4 w-4" /> Deploy issuer
          </Button>
        </CardHeader>
        <CardContent>
          {clones === null ? (
            <div className="flex justify-center py-8">
              <Spinner className="h-5 w-5 text-muted" />
            </div>
          ) : clones.length === 0 ? (
            <p className="py-8 text-center text-sm text-muted">
              {clonesError
                ? "Clone activity is unavailable — the oversight indexer is not wired (INDEXER_API_BASE). Deploys and the address preview still work."
                : "No issuer clones deployed yet."}
            </p>
          ) : (
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>Clone</TableHead>
                  <TableHead>Name</TableHead>
                  <TableHead>Record type</TableHead>
                  <TableHead className="text-right">Issued</TableHead>
                  <TableHead className="text-right">Revoked</TableHead>
                  <TableHead className="text-right">Active</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {clones.map((c) => (
                  <TableRow key={c.clone}>
                    <TableCell>
                      <AddrLink addr={c.clone} />
                    </TableCell>
                    <TableCell className="font-medium">{c.cloneName ?? c.name ?? "—"}</TableCell>
                    <TableCell className="font-mono text-xs">{c.recordType ?? "—"}</TableCell>
                    <TableCell className="text-right tabular-nums">{c.issued}</TableCell>
                    <TableCell className="text-right tabular-nums">{c.revoked}</TableCell>
                    <TableCell className="text-right tabular-nums">{c.active}</TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          )}
        </CardContent>
      </Card>

      <DeployDialog
        open={open}
        onOpenChange={setOpen}
        onDeployed={(r) => {
          setResult(r);
          void loadClones();
        }}
      />

      <DeployResultDialog result={result} onClose={() => setResult(null)} />
    </div>
  );
}

/** Surfaces WHO owns the factory + whether a deploy from the hosted key executes directly or proposes. */
function AuthorityBanner({ authority }: { authority: GovernanceAuthorityResp | null }) {
  if (!authority) return null;
  const fo = authority.factoryOwner;
  // heldByHosted: true → direct broadcast; false → proposed to the governance owner; null → unknown.
  const held = fo.heldByHosted;
  return (
    <Card>
      <CardContent className="flex flex-col gap-3 py-4 sm:flex-row sm:items-center sm:justify-between">
        <div className="space-y-1 text-sm">
          <div className="flex items-center gap-2 font-medium text-onSurface">
            <ShieldCheck className="h-4 w-4 text-primary" /> Factory owner
          </div>
          <div className="text-muted">
            {fo.owner ? <AddrLink addr={fo.owner} /> : <span>unavailable</span>}
            {fo.pendingOwner && (
              <span className="ml-2 text-xs">
                (pending transfer → <AddrLink addr={fo.pendingOwner} />)
              </span>
            )}
          </div>
          {authority.hostedSigner && (
            <div className="text-xs text-muted">
              Hosted signer: <AddrLink addr={authority.hostedSigner} />
            </div>
          )}
        </div>
        {held === true ? (
          <Badge variant="success">
            <CheckCircle2 className="h-3 w-3" /> Hosted key deploys directly
          </Badge>
        ) : held === false ? (
          <Badge variant="warning">
            <Send className="h-3 w-3" /> Deploys route to governance as proposals
          </Badge>
        ) : (
          <Badge variant="neutral">owner unknown</Badge>
        )}
      </CardContent>
    </Card>
  );
}

function DeployDialog({
  open,
  onOpenChange,
  onDeployed,
}: {
  open: boolean;
  onOpenChange: (o: boolean) => void;
  onDeployed: (r: CreateIssuerResp) => void;
}) {
  const { central } = useApp();
  const { toast } = useToast();
  const [name, setName] = useState("");
  const [recordType, setRecordType] = useState("");
  const [business, setBusiness] = useState("");
  const [predicted, setPredicted] = useState<string | null>(null);
  const [predicting, setPredicting] = useState(false);
  const [predictError, setPredictError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const seq = useRef(0);

  // Reset the form each time the dialog opens.
  useEffect(() => {
    if (open) {
      setName("");
      setRecordType("");
      setBusiness("");
      setPredicted(null);
      setPredictError(null);
    }
  }, [open]);

  // Live deterministic clone-address preview: debounce (recordType, business) and call predictIssuer.
  // The address is fixed by salt = keccak256(recordType, business), so the preview is exact. `business`
  // is optional — the backend defaults it to the hosted signer (single-authority topology).
  useEffect(() => {
    if (!open) return;
    const rt = recordType.trim();
    if (!rt) {
      setPredicted(null);
      setPredictError(null);
      return;
    }
    const biz = business.trim();
    const mine = ++seq.current;
    setPredicting(true);
    const t = setTimeout(() => {
      central
        .predictIssuer({ recordType: rt, business: biz || undefined })
        .then((r) => {
          if (mine !== seq.current) return; // a newer keystroke superseded this request
          setPredicted(r.predicted);
          setPredictError(null);
        })
        .catch((err: Error) => {
          if (mine !== seq.current) return;
          setPredicted(null);
          setPredictError(err.message);
        })
        .finally(() => {
          if (mine === seq.current) setPredicting(false);
        });
    }, 350);
    return () => clearTimeout(t);
  }, [open, recordType, business, central]);

  async function submit(e: FormEvent) {
    e.preventDefault();
    setBusy(true);
    try {
      const r = await central.createIssuer({
        name: name.trim(),
        recordType: recordType.trim(),
        business: business.trim() || undefined,
      });
      onDeployed(r);
      onOpenChange(false);
      toast({
        title:
          r.result.disposition === "executed" ? "Issuer deployed" : "Deploy proposed to governance",
        variant: "success",
      });
    } catch (err) {
      toast({ title: "Deploy failed", description: (err as Error).message, variant: "danger" });
    } finally {
      setBusy(false);
    }
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-lg">
        <DialogHeader>
          <DialogTitle>Deploy an issuer clone</DialogTitle>
          <DialogDescription>
            Deploys <code>factory.createIssuer(name, keccak256(recordType), business)</code> through
            the governance-action layer. The clone address is deterministic and shown live below.
          </DialogDescription>
        </DialogHeader>
        {env.demoMode && (
          <div>
            <Button
              type="button"
              variant="outline"
              size="sm"
              onClick={() => {
                const preset = demoIssuerDeploy();
                setName(preset.name);
                setRecordType(preset.recordType);
                setBusiness(preset.business);
              }}
            >
              <Sparkles className="h-4 w-4" /> Fill demo data
            </Button>
          </div>
        )}
        <form onSubmit={submit} className="space-y-3">
          <div className="space-y-1.5">
            <Label required>Name</Label>
            <Input
              value={name}
              onChange={(e) => setName(e.target.value)}
              required
              placeholder="Seaport Vaccination Authority"
            />
          </div>
          <div className="space-y-1.5">
            <Label required>Record type</Label>
            <Input
              value={recordType}
              onChange={(e) => setRecordType(e.target.value)}
              required
              placeholder="VACCINATION"
            />
            <p className="text-xs text-muted">
              A human label (hashed server-side) or a raw <code>0x</code>+64-hex key. Part of the
              deterministic salt.
            </p>
          </div>
          <div className="space-y-1.5">
            <Label>Business (optional)</Label>
            <Input
              value={business}
              onChange={(e) => setBusiness(e.target.value)}
              placeholder="0x… (defaults to the hosted signer)"
            />
            <p className="text-xs text-muted">
              The <code>business</code> salt component. Leave blank for the single-authority topology
              (defaults to the hosted signer, matching the deployed clones).
            </p>
          </div>

          <div className="rounded-md border border-dashed border-border bg-surface-muted p-3">
            <div className="flex items-center justify-between gap-2">
              <span className="text-xs font-medium uppercase tracking-wide text-muted">
                Predicted clone address
              </span>
              {predicting && <Spinner className="h-4 w-4 text-muted" />}
            </div>
            {predicted ? (
              <div className="mt-1 flex items-center gap-2">
                <AddressRef
                  address={predicted}
                  full
                  className="break-all text-sm"
                  testId="issuers-predicted"
                />
                <CopyButton value={predicted} label="predicted clone address" />
              </div>
            ) : predictError ? (
              <p className="mt-1 text-xs text-danger">{predictError}</p>
            ) : (
              <p className="mt-1 text-xs text-muted">Enter a record type to preview the address.</p>
            )}
          </div>

          <Button type="submit" loading={busy} disabled={!name.trim() || !recordType.trim()}>
            <Factory className="h-4 w-4" /> Deploy
          </Button>
        </form>
      </DialogContent>
    </Dialog>
  );
}

/** Shows the deploy outcome: an executed tx (with explorer link) or a governance proposal payload. */
function DeployResultDialog({
  result,
  onClose,
}: {
  result: CreateIssuerResp | null;
  onClose: () => void;
}) {
  return (
    <Dialog open={result !== null} onOpenChange={(o) => !o && onClose()}>
      <DialogContent className="max-w-lg">
        {result && (
          <>
            <DialogHeader>
              <DialogTitle>
                {result.result.disposition === "executed"
                  ? "Issuer deployed"
                  : "Deploy proposed to governance"}
              </DialogTitle>
              <DialogDescription>
                {result.result.disposition === "executed"
                  ? "The hosted key held factory ownership — createIssuer was signed and broadcast."
                  : "Factory ownership sits with the governance signer. Nothing was broadcast; hand the calldata below to the owner to execute."}
              </DialogDescription>
            </DialogHeader>
            <div className="space-y-2 text-sm">
              <Row label="Predicted clone" value={result.predicted} link="address" mono />
              <Row label="Record type key" value={result.recordTypeKey} mono />
              <Row label="Business (salt)" value={result.business} mono />
              {result.result.disposition === "executed" ? (
                <Row label="Deploy tx" value={result.result.txHash} link="tx" mono />
              ) : (
                <>
                  {result.result.holder && (
                    <Row label="Owner (execute as)" value={result.result.holder} link="address" mono />
                  )}
                  <Row label="Target" value={result.result.target} link="address" mono />
                  <Row label="Authority" value={result.result.authority} />
                  <Row label="Calldata" value={result.result.calldata} mono />
                </>
              )}
            </div>
          </>
        )}
      </DialogContent>
    </Dialog>
  );
}

function Row({
  label,
  value,
  link,
  mono,
}: {
  label: string;
  value: string;
  link?: "tx" | "address";
  mono?: boolean;
}) {
  // A row is linked only when the value can actually be looked up. `link` says what KIND of thing this
  // is, never that a link is due: it used to be read as the latter, so a `Deploy tx` row offered an
  // explorer anchor for whatever string the backend returned. A row with no `link` kind is plain text
  // and claims nothing (`Calldata`, `Authority`), which is why its reason stays null.
  const { href, reason } =
    link === "tx"
      ? txChainRef({ txHash: value })
      : link === "address"
        ? addressChainRef(value)
        : { href: null, reason: null };
  return (
    <div className="flex items-center justify-between gap-2 rounded-md border border-border px-3 py-2">
      <div className="min-w-0">
        <div className="text-xs text-muted">{label}</div>
        {href ? (
          <a
            href={href}
            target="_blank"
            rel="noreferrer"
            className={`block truncate text-primary hover:underline ${mono ? "font-mono text-xs" : ""}`}
          >
            {value}
          </a>
        ) : (
          <div
            className={`truncate ${mono ? "font-mono text-xs" : ""}`}
            title={reason ? `${value} — ${reason}` : undefined}
            data-reason={reason ?? undefined}
          >
            {value}
          </div>
        )}
        {reason && (
          <div className="text-[10px] text-warning" data-testid="issuers-row-unlinkable">
            {reason}
          </div>
        )}
      </div>
      <Button size="sm" variant="ghost" onClick={() => void navigator.clipboard?.writeText(value)}>
        <Copy className="h-4 w-4" />
      </Button>
    </div>
  );
}

/** A short address, explorer-linked only when it can be looked up. */
function AddrLink({ addr }: { addr: string }) {
  return <AddressRef address={addr} testId="issuers-address" />;
}
