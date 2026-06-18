import {
  Badge,
  Button,
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
  Input,
  Label,
  useToast,
  type FragmentState,
  type ImportVerdict,
} from "@dogtag/ui";
import { CheckCircle2, HelpCircle, ScanLine, XCircle } from "lucide-react";
import { useState, type FormEvent } from "react";
import { useApp } from "../app/AppContext";

/**
 * Import a customer's pet PROFILE / VACCINATION credential (impl §5.2 / §3.5) — off-chain,
 * DECOUPLED from Verify. The customer app shows a QR carrying { userApiBase, userJwt, recordRef };
 * the groomer scans it, pulls the doc, and the backend third-party-verifies it on chain + DNS
 * (the three authenticity pillars). The verdict is shown before the record is accepted.
 */
export function ImportFromUser() {
  const { api } = useApp();
  const { toast } = useToast();
  const [kind, setKind] = useState<"profile" | "vaccination">("vaccination");
  const [userApiBase, setUserApiBase] = useState("");
  const [userJwt, setUserJwt] = useState("");
  const [recordRef, setRecordRef] = useState("");
  const [busy, setBusy] = useState(false);
  const [verdict, setVerdict] = useState<ImportVerdict | null>(null);
  const [accepted, setAccepted] = useState<boolean | null>(null);

  function tryParseScanned(text: string) {
    // a scanned QR may encode the whole payload as JSON; accept that too.
    try {
      const obj = JSON.parse(text) as Partial<{
        userApiBase: string;
        userJwt: string;
        recordRef: string;
      }>;
      if (obj.userApiBase) setUserApiBase(obj.userApiBase);
      if (obj.userJwt) setUserJwt(obj.userJwt);
      if (obj.recordRef) setRecordRef(obj.recordRef);
    } catch {
      /* not JSON — treat as a recordRef */
      setRecordRef(text);
    }
  }

  async function submit(e: FormEvent) {
    e.preventDefault();
    setBusy(true);
    setVerdict(null);
    setAccepted(null);
    try {
      const r = await api.importPull({ userApiBase, userJwt, recordRef });
      setVerdict((r.verdict as ImportVerdict) ?? null);
      setAccepted(r.imported);
      toast({
        title: r.imported ? `${kind} accepted` : "Not accepted",
        description: r.imported
          ? "On-chain + DNS verification passed."
          : "Verification failed — the record was not accepted.",
        variant: r.imported ? "success" : "danger",
      });
    } catch (err) {
      // backend returns 422 with a verdict when verification fails; surface it.
      const body = (err as { body?: unknown }).body;
      if (body && typeof body === "object" && "verdict" in body) {
        setVerdict((body as { verdict: ImportVerdict }).verdict);
        setAccepted(false);
      }
      toast({ title: "Import failed", description: (err as Error).message, variant: "danger" });
    } finally {
      setBusy(false);
    }
  }

  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex items-center gap-2">
          <ScanLine className="h-5 w-5 text-primary" /> Import from customer
        </CardTitle>
        <CardDescription>
          Pull a {kind} credential from the customer's app and verify it on chain + DNS BEFORE
          accepting (off-chain; decoupled from Verify).
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-4">
        <div className="flex gap-2">
          <Button
            variant={kind === "profile" ? "primary" : "outline"}
            size="sm"
            onClick={() => setKind("profile")}
          >
            Import Profile
          </Button>
          <Button
            variant={kind === "vaccination" ? "primary" : "outline"}
            size="sm"
            onClick={() => setKind("vaccination")}
          >
            Import Vaccination
          </Button>
        </div>

        <div className="rounded-lg border border-dashed border-border bg-surface-muted p-4 text-sm text-muted">
          Ask the customer to open their DogTag app and present the share QR. Scan it, or paste the
          scanned payload / record reference below.
        </div>

        <div className="space-y-1.5">
          <Label>Scanned payload</Label>
          <Input
            placeholder="Paste scanned QR contents…"
            onChange={(e) => tryParseScanned(e.target.value)}
          />
        </div>

        <form onSubmit={submit} className="grid gap-4 sm:grid-cols-2">
          <div className="space-y-1.5">
            <Label required>Customer API base</Label>
            <Input
              value={userApiBase}
              onChange={(e) => setUserApiBase(e.target.value)}
              required
              placeholder="https://api.dogtag.io"
            />
          </div>
          <div className="space-y-1.5">
            <Label required>Record reference</Label>
            <Input value={recordRef} onChange={(e) => setRecordRef(e.target.value)} required />
          </div>
          <div className="space-y-1.5 sm:col-span-2">
            <Label required>Customer JWT</Label>
            <Input value={userJwt} onChange={(e) => setUserJwt(e.target.value)} required />
          </div>
          <div className="sm:col-span-2">
            <Button type="submit" loading={busy}>
              Pull &amp; verify
            </Button>
          </div>
        </form>

        {verdict && <VerdictPanel verdict={verdict} accepted={accepted} />}
      </CardContent>
    </Card>
  );
}

function VerdictPanel({
  verdict,
  accepted,
}: {
  verdict: ImportVerdict;
  accepted: boolean | null;
}) {
  return (
    <div className="space-y-3 rounded-lg border border-border p-4">
      <div className="flex items-center justify-between">
        <span className="text-sm font-semibold text-onSurface">Verification verdict</span>
        <Badge variant={verdict.valid ? "success" : "danger"}>
          {verdict.valid ? "VALID" : "INVALID"}
        </Badge>
      </div>
      <p className="text-xs text-muted">
        The three authenticity pillars define validity for everyone. <em>Ownership</em> is a
        contextual fourth fragment — <code>NOT_APPLICABLE</code> for a third-party groomer importing
        a customer's record.
      </p>
      <div className="grid gap-2 sm:grid-cols-2">
        <Pillar label="Integrity" state={verdict.integrity} />
        <Pillar label="Issuance" state={verdict.issuance} />
        <Pillar label="Identity (DNS)" state={verdict.identity} />
        <Pillar label="Ownership" state={verdict.ownership} />
      </div>
      {accepted !== null && (
        <p className="text-sm">
          {accepted ? (
            <span className="text-success">Record accepted and imported.</span>
          ) : (
            <span className="text-danger">Record rejected — not imported.</span>
          )}
        </p>
      )}
    </div>
  );
}

function Pillar({ label, state }: { label: string; state: FragmentState }) {
  const map: Record<
    FragmentState,
    { variant: "success" | "danger" | "warning" | "neutral"; icon: typeof CheckCircle2 }
  > = {
    VALID: { variant: "success", icon: CheckCircle2 },
    INVALID: { variant: "danger", icon: XCircle },
    ERROR: { variant: "warning", icon: HelpCircle },
    NOT_APPLICABLE: { variant: "neutral", icon: HelpCircle },
  };
  const { variant, icon: Icon } = map[state];
  return (
    <div className="flex items-center justify-between gap-2 rounded-md border border-border px-3 py-2">
      <span className="text-sm text-onSurface">{label}</span>
      <Badge variant={variant}>
        <Icon className="h-3 w-3" />
        {state}
      </Badge>
    </div>
  );
}
