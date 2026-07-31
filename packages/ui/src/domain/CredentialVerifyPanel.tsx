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
  /**
   * IssuerRegistry address, forwarded but DELIBERATELY IGNORED by the whitelist pillar.
   *
   * A clone is gated by its OWN `registry()`, and `IssuerRegistry._wl` and its `Whitelisted`/
   * `Delisted` events are per-CONTRACT - so only that instance's log can answer for this contract's
   * issuances. The pillar therefore reads the authority off the clone the factory resolved. Honouring
   * an override here would let a mis-paired client find no grant in a registry that governs nothing
   * and print a definite refusal of a genuine credential: our own misconfiguration rendered as an
   * accusation. See `verifyCredentialOnchain`'s `registryAddr` for the same note at the seam itself.
   *
   * Kept on the props because the value is still meaningful to OTHER surfaces - the bench's
   * `registry-governs-issuer` row compares it against the governing one to diagnose a mis-paired
   * factory/registry deployment - and it is passed through so this panel and that row see one value.
   */
  registryAddr?: string;
  /**
   * DogTagIssuerFactory address used to resolve the issuing clone from its write-once `rootIssuer[R]`
   * index; defaults to the deployed ROAX factory. This one really is load-bearing: it is the anchor
   * every on-chain read is made against, and unlike
   * {@link CredentialVerifyPanelProps.registryAddr} it is never sourced from the document.
   */
  factoryAddr?: string;
  /** Public RPC URL override; defaults to the ROAX devrpc from the chain definition. */
  rpcUrl?: string;
  /** Bundled endpoint to use, after its own chain guard, if the preferred endpoint cannot be used. */
  defaultRpcUrl?: string;
  /** Injected chain reader (tests/storybook); defaults to a viem reader over the public ROAX RPC. */
  reader?: IssuerChainReader;
}

// Every status the API can return needs a label here: an unmapped one renders blank, which reads as
// "nothing wrong" for exactly the states that ARE something wrong. The `issuer_*` arms come from the
// factory-anchored issuer pillar, and each says something different - "the envelope names the wrong
// contract", "the signer is not authorised", "we could not establish who issued this at all".
const STATUS_LABEL: Record<VerifyCredentialResp["status"], string> = {
  valid: "Valid",
  revoked: "Revoked",
  not_issued: "Not issued",
  integrity_failed: "Integrity failed",
  invalid: "Invalid",
  issuer_mismatch: "Issuer mismatch",
  issuer_not_whitelisted: "Issuer not authorised",
  issuer_unresolved: "Issuer unverified",
};

export function CredentialVerifyPanel({
  defaultSigner = "",
  registryAddr,
  factoryAddr,
  rpcUrl,
  defaultRpcUrl,
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
        factoryAddr,
        rpcUrl,
        defaultRpcUrl,
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
          and revocation. Permissionless - checked in-browser through your chain-guarded endpoint
          selection, with the bundled endpoint as a guarded fallback and no operator session
          required.
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
            <Label>Expected issuer signer</Label>
            <Input
              value={signer}
              onChange={(e) => setSigner(e.target.value)}
              placeholder="0x... optional"
            />
            <p className="text-xs text-muted">
              Optional. The issuer whitelist is always checked - the signer is read from the chain
              (<code>issuedBy</code>), never typed in. Fill this only to additionally require that the
              credential was issued by one specific address.
            </p>
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
  // `status` describes the on-chain RECORD (anchored? revoked?); `verdict` is whether the credential
  // as a whole verified. A record can be `valid` on-chain while the credential fails - an unresolved
  // or non-whitelisted issuer is exactly that case. Never paint the status chip green then, or the
  // loud signal contradicts the true one.
  const statusVariant = !result.verdict
    ? result.status === "revoked"
      ? "danger"
      : "warning"
    : result.status === "valid"
      ? "success"
      : result.status === "revoked"
        ? "danger"
        : "warning";
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
        {/* "at issuance": the pillar reads the governing registry's grant LOG at the anchoring
            block, not `isWhitelistedFor`. Delisting is forward-only, so a green tile beside a signer
            an operator knows is delisted today is correct, and the label has to say why. */}
        <Pillar label="Issuer authorised at issuance" value={result.fragments.issuerWhitelisted} />
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
        <Detail
          label="Issuing signer (from chain)"
          value={result.signerAddr || "unresolved - this clone never issued this root"}
        />
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
  // An indeterminate pillar is a FAILURE to establish the claim, not an optional step that was
  // skipped - it cannot contribute to a pass, so it must not be styled like a neutral "n/a".
  if (value === null || value === undefined) {
    return (
      <div className="rounded-md border border-border bg-surface px-3 py-2">
        <div className="text-xs text-muted">{label}</div>
        <div className="text-sm font-semibold text-warning">Unresolved</div>
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
