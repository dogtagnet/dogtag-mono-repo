import { Badge, Button, Card } from "@dogtag/ui";
import { useEffect, useState } from "react";
import { apiPost, type Health } from "../lib/api";

function Frag({ label, v, testid }: { label: string; v: unknown; testid?: string }) {
  if (v === null || v === undefined)
    return (
      <Badge data-testid={testid} variant="neutral">
        {label}: n/a
      </Badge>
    );
  return (
    <Badge data-testid={testid} variant={v ? "success" : "danger"}>
      {label}: {v ? "yes" : "no"}
    </Badge>
  );
}

export function Verify({ health }: { health: Health | null }) {
  const [doc, setDoc] = useState("");
  const [signer, setSigner] = useState("");
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<Record<string, unknown> | null>(null);
  const [error, setError] = useState<string | null>(null);

  // Prefill the issuer signer with this authority's own signer so the whitelist pillar (the third
  // authenticity pillar) is exercised by default. Users can override or clear it.
  useEffect(() => {
    if (health?.signer && !signer) setSigner(health.signer);
  }, [health?.signer]);

  async function submit() {
    setBusy(true);
    setError(null);
    setResult(null);
    try {
      const wrapped = JSON.parse(doc);
      const body: Record<string, unknown> = { wrapped_doc: wrapped };
      if (signer.trim()) body.signer_addr = signer.trim();
      const { status, json } = await apiPost("/v1/verify", body);
      if (status !== 200) setError(json.error || `HTTP ${status}`);
      else setResult(json);
    } catch (e) {
      setError(`Invalid JSON or request: ${e}`);
    } finally {
      setBusy(false);
    }
  }

  const frag = result?.fragments as Record<string, unknown> | undefined;
  const verdict = result?.verdict as boolean | undefined;

  return (
    <Card className="p-6">
      <h2 className="text-base font-semibold text-onSurface">Verify a credential</h2>
      <p className="mt-1 text-sm text-muted">
        Recomputes integrity (offline) and reads on-chain status (DogTagIssuer.isValid) + issuer
        identity (IssuerRegistry.isWhitelistedFor) from ROAX. All reads are gasless.
      </p>

      <label className="mt-4 block text-xs font-medium text-muted">
        Wrapped credential document (JSON)
      </label>
      <textarea
        data-testid="verify-doc"
        placeholder="Paste the wrappedDoc returned by an issuer…"
        value={doc}
        onChange={(e) => setDoc(e.target.value)}
        className="receipt-mono mt-1 min-h-[140px] w-full rounded-md border border-input bg-surface px-3 py-2 text-xs text-onSurface focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
      />

      <label className="mt-4 block text-xs font-medium text-muted">
        Issuer signer address (optional — checks the whitelist pillar)
      </label>
      <input
        data-testid="verify-signer"
        value={signer}
        onChange={(e) => setSigner(e.target.value)}
        placeholder="0x…"
        className="mt-1 flex h-10 w-full rounded-md border border-input bg-surface px-3 text-sm text-onSurface focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
      />

      <div className="mt-4">
        <Button data-testid="verify-submit" loading={busy} disabled={busy || !doc.trim()} onClick={submit}>
          {busy ? "Verifying…" : "Verify"}
        </Button>
      </div>

      {error && (
        <p className="mt-4">
          <Badge variant="danger">{error}</Badge>
        </p>
      )}

      {result && (
        <div className="mt-5">
          <div
            data-testid="verdict"
            className={`text-lg font-bold ${verdict ? "ok text-success" : "bad text-danger"}`}
          >
            {verdict ? "✓ VALID" : "✗ INVALID"}
          </div>
          <div className="mt-3 flex flex-wrap gap-2">
            <Frag label="integrity" testid="pillar-integrity" v={frag?.integrity} />
            <Frag label="on-chain" testid="pillar-onchain" v={frag?.onchain} />
            <Frag label="issuer whitelist" testid="pillar-whitelist" v={frag?.issuerWhitelisted} />
          </div>
          <pre className="receipt-mono mt-4 max-h-80 overflow-auto rounded-md border border-border bg-surface-muted p-3 text-xs text-onSurface">
            {JSON.stringify(result, null, 2)}
          </pre>
        </div>
      )}
    </Card>
  );
}
