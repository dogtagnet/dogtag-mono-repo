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
} from "@dogtag/ui";
import { ScanLine } from "lucide-react";
import { useState, type FormEvent } from "react";
import { useApp } from "../app/AppContext";

/**
 * Import a user's profile / vaccination credential (impl §3.5) — off-chain, DECOUPLED from Verify.
 * The user app shows a QR carrying { userApiBase, userJwt, recordRef }; the operator scans it and
 * pulls + third-party-verifies the doc. This UI accepts a pasted/scanned payload.
 */
export function ImportFromUser() {
  const { api } = useApp();
  const { toast } = useToast();
  const [kind, setKind] = useState<"profile" | "vaccination">("vaccination");
  const [userApiBase, setUserApiBase] = useState("");
  const [userJwt, setUserJwt] = useState("");
  const [recordRef, setRecordRef] = useState("");
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<unknown>(null);

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
    setResult(null);
    try {
      const r = await api.importPull({ userApiBase, userJwt, recordRef });
      setResult(r.verdict);
      toast({ title: r.imported ? "Imported" : "Not imported", variant: r.imported ? "success" : "danger" });
    } catch (err) {
      toast({ title: "Import failed", description: (err as Error).message, variant: "danger" });
    } finally {
      setBusy(false);
    }
  }

  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex items-center gap-2">
          <ScanLine className="h-5 w-5 text-primary" /> Import from user
        </CardTitle>
        <CardDescription>
          Pull a {kind} credential from the user's app and third-party-verify it (off-chain; decoupled from Verify).
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-4">
        <div className="flex gap-2">
          <Button variant={kind === "profile" ? "primary" : "outline"} size="sm" onClick={() => setKind("profile")}>
            Import Profile
          </Button>
          <Button variant={kind === "vaccination" ? "primary" : "outline"} size="sm" onClick={() => setKind("vaccination")}>
            Import Vaccination
          </Button>
        </div>

        <div className="rounded-lg border border-dashed border-border bg-surface-muted p-4 text-sm text-muted">
          Ask the user to open their DogTag app and present the share QR. Scan it, or paste the scanned
          payload / record reference below.
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
            <Label required>User API base</Label>
            <Input value={userApiBase} onChange={(e) => setUserApiBase(e.target.value)} required placeholder="https://api.dogtag.io" />
          </div>
          <div className="space-y-1.5">
            <Label required>Record reference</Label>
            <Input value={recordRef} onChange={(e) => setRecordRef(e.target.value)} required />
          </div>
          <div className="space-y-1.5 sm:col-span-2">
            <Label required>User JWT</Label>
            <Input value={userJwt} onChange={(e) => setUserJwt(e.target.value)} required />
          </div>
          <div className="sm:col-span-2">
            <Button type="submit" loading={busy}>
              Pull &amp; verify
            </Button>
          </div>
        </form>

        {result !== null && (
          <div className="space-y-2">
            <Badge variant="success">verified</Badge>
            <pre className="overflow-x-auto rounded-md border border-border bg-surface-muted p-3 text-xs">
              {JSON.stringify(result, null, 2)}
            </pre>
          </div>
        )}
      </CardContent>
    </Card>
  );
}
