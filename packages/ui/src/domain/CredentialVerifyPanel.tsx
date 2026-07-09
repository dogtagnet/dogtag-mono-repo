import { CircleAlert, ClipboardCheck, RotateCcw, ShieldCheck } from "lucide-react";
import { useState, type FormEvent } from "react";
import type { VerifyCredentialResp } from "../api/types";
import { Badge } from "../components/Badge";
import { Button } from "../components/Button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "../components/Card";
import { Input } from "../components/Input";
import { Label } from "../components/Label";
import { useToast } from "../components/Toast";
import { explorerAddressUrl } from "../wallet/chain";
import { verifyCredentialOnchain, type IssuerChainReader } from "../wallet/verifyCredential";

export interface CredentialVerifyPanelProps {
  defaultSigner?: string;
  /** IssuerRegistry address for the whitelist pillar; defaults to the deployed ROAX registry. */
  registryAddr?: string;
  /** Public RPC URL override; defaults to the ROAX devrpc from the chain definition. */
  rpcUrl?: string;
  /** Injected chain reader (tests/storybook); defaults to a viem reader over the public ROAX RPC. */
  reader?: IssuerChainReader;
}

const STATUS_LABEL: Record<VerifyCredentialResp["status"], string> = {
  valid: "Valid",
  revoked: "Revoked",
  not_issued: "Not issued",
  integrity_failed: "Integrity failed",
  invalid: "Invalid",
};

export function CredentialVerifyPanel({
  defaultSigner = "",
  registryAddr,
  rpcUrl,
  reader,
}: CredentialVerifyPanelProps = {}) {
  const { toast } = useToast();
  const [doc, setDoc] = useState("");
  const [signer, setSigner] = useState(defaultSigner);
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<VerifyCredentialResp | null>(null);

  async function submit(e: FormEvent) {
    e.preventDefault();
    setBusy(true);
    setResult(null);
    try {
      const parsed = JSON.parse(doc) as unknown;
      if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) {
        throw new Error("Wrapped document must be a JSON object.");
      }
      // Permissionless direct-to-RPC verification (same on-chain reads the mobile apps do); no
      // operator-gated endpoint required.
      const resp = await verifyCredentialOnchain({
        wrappedDoc: parsed as Record<string, unknown>,
        signerAddr: signer.trim() || undefined,
        registryAddr,
        rpcUrl,
        reader,
      });
      setResult(resp);
    } catch (err) {
      toast({ title: "Verify failed", description: (err as Error).message, variant: "danger" });
    } finally {
      setBusy(false);
    }
  }

  function reset() {
    setDoc("");
    setSigner(defaultSigner);
    setResult(null);
  }

  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex items-center gap-2">
          <ShieldCheck className="h-5 w-5 text-primary" /> Check credential status
        </CardTitle>
        <CardDescription>
          Recompute integrity and read the DogTag issuer clone directly on ROAX for current validity
          and revocation. Permissionless - verified in-browser over the public RPC, no operator
          session required.
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-5">
        <form onSubmit={submit} className="space-y-4">
          <div className="space-y-1.5">
            <Label required>Wrapped credential document</Label>
            <textarea
              value={doc}
              onChange={(e) => setDoc(e.target.value)}
              placeholder="Paste wrappedDoc JSON"
              className="min-h-[156px] w-full rounded-md border border-input bg-surface px-3 py-2 font-mono text-xs text-onSurface focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
            />
          </div>

          <div className="space-y-1.5">
            <Label>Issuer signer address</Label>
            <Input
              value={signer}
              onChange={(e) => setSigner(e.target.value)}
              placeholder="0x... optional"
            />
          </div>

          <div className="flex flex-wrap items-center gap-2">
            <Button type="submit" loading={busy} disabled={!doc.trim()}>
              <ClipboardCheck className="h-4 w-4" /> Verify credential
            </Button>
            {(doc || result) && (
              <Button type="button" variant="ghost" onClick={reset}>
                <RotateCcw className="h-4 w-4" /> Reset
              </Button>
            )}
          </div>
        </form>

        {result && <CredentialVerifyResult result={result} />}
      </CardContent>
    </Card>
  );
}

function CredentialVerifyResult({ result }: { result: VerifyCredentialResp }) {
  const statusVariant =
    result.status === "valid" ? "success" : result.status === "revoked" ? "danger" : "warning";
  const issuedAt = formatIssuedAt(result.issuedAt);
  return (
    <div className="space-y-4 rounded-md border border-border bg-surface-muted p-4">
      <div className="flex flex-wrap items-center gap-2">
        <Badge variant={result.verdict ? "success" : "danger"}>
          {result.verdict ? (
            <ShieldCheck className="h-3.5 w-3.5" />
          ) : (
            <CircleAlert className="h-3.5 w-3.5" />
          )}
          {result.verdict ? "Verdict: pass" : "Verdict: fail"}
        </Badge>
        <Badge variant={statusVariant}>{STATUS_LABEL[result.status]}</Badge>
        <Badge variant="neutral">{result.recordType}</Badge>
      </div>

      <div className="grid gap-2 text-sm sm:grid-cols-2">
        <Pillar label="Integrity" value={result.fragments.integrity} />
        <Pillar label="On-chain valid" value={result.fragments.onchain} />
        <Pillar label="Issued" value={result.fragments.issued} />
        <Pillar label="Revoked" value={result.fragments.revoked} invert />
        <Pillar label="Issuer whitelist" value={result.fragments.issuerWhitelisted} />
      </div>

      <dl className="space-y-2 text-sm">
        <Detail label="Root" value={result.root} />
        <Detail label="Recomputed root" value={result.recomputedRoot} />
        <div className="break-all">
          <dt className="text-muted">Issuer clone</dt>
          <dd>
            <a
              href={explorerAddressUrl(result.issuerAddr)}
              target="_blank"
              rel="noreferrer"
              className="font-mono text-primary hover:underline"
            >
              {result.issuerAddr}
            </a>
          </dd>
        </div>
        <Detail label="Issued at" value={issuedAt} />
      </dl>
    </div>
  );
}

function Pillar({
  label,
  value,
  invert = false,
}: {
  label: string;
  value?: boolean | null;
  invert?: boolean;
}) {
  if (value === null || value === undefined) {
    return (
      <div className="rounded-md border border-border bg-surface px-3 py-2">
        <div className="text-xs text-muted">{label}</div>
        <div className="text-sm font-semibold text-muted">Not checked</div>
      </div>
    );
  }
  const pass = invert ? !value : value;
  return (
    <div className="rounded-md border border-border bg-surface px-3 py-2">
      <div className="text-xs text-muted">{label}</div>
      <div className={pass ? "text-sm font-semibold text-success" : "text-sm font-semibold text-danger"}>
        {value ? "Yes" : "No"}
      </div>
    </div>
  );
}

function Detail({ label, value }: { label: string; value: string }) {
  return (
    <div className="break-all">
      <dt className="text-muted">{label}</dt>
      <dd className="font-mono">{value}</dd>
    </div>
  );
}

function formatIssuedAt(value: string) {
  const n = Number.parseInt(value, 10);
  if (!Number.isFinite(n) || n <= 0) return "not issued";
  return new Date(n * 1000).toISOString();
}
