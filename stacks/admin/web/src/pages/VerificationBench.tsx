import {
  BENCH_MUTATIONS,
  Badge,
  Button,
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
  Input,
  Label,
  runVerificationBench,
  useToast,
  type BenchCheck,
  type BenchMutation,
  type BenchReport,
  type CheckOutcome,
} from "@dogtag/ui";
import { AlertTriangle, CheckCircle2, FlaskConical, HelpCircle, Upload, XCircle } from "lucide-react";
import { useRef, useState, type ChangeEvent } from "react";
import { env } from "../lib/env";

/**
 * The verification bench: throw any record at the real verifier and watch exactly which checks pass,
 * which fail, and which could not run.
 *
 * # Admin-only, deliberately
 *
 * This surface shows provenance an ordinary verify result does not: which contract answered, at which
 * block, and why an unresolved pillar was unresolved. That is operator diagnostics, not a public
 * oracle, so it lives behind the admin token like every other page here (`App.tsx` renders `Login`
 * until one exists). It performs no writes and no deploys - every chain access is a gasless read.
 *
 * # Nothing on this page is computed by this page
 *
 * The verdict and every check outcome come from `runVerificationBench`, which runs the SAME
 * `verifyCredentialOnchain` the wallet and the credential verify panel run, and observes its reads.
 * This file only renders. That separation is the point: a bench that decided its own answers would
 * agree with itself and prove nothing.
 */

const OUTCOME_STYLE: Record<
  CheckOutcome,
  { label: string; variant: "success" | "danger" | "neutral"; Icon: typeof CheckCircle2 }
> = {
  pass: { label: "Pass", variant: "success", Icon: CheckCircle2 },
  fail: { label: "Fail", variant: "danger", Icon: XCircle },
  // NEUTRAL, never a softened red and never a green. A check that could not run is not a weak failure
  // and not a qualified pass - rendering it as either is the defect this whole surface exists to close.
  "could-not-run": { label: "Could not run", variant: "neutral", Icon: HelpCircle },
};

function CheckRow({ check }: { check: BenchCheck }) {
  const { label, variant, Icon } = OUTCOME_STYLE[check.outcome];
  return (
    <div className="border-b border-border py-4 last:border-b-0" data-testid={`check-${check.id}`}>
      <div className="flex flex-wrap items-start justify-between gap-3">
        <p className="max-w-2xl text-sm font-medium text-onSurface">{check.question}</p>
        <Badge variant={variant} data-testid={`outcome-${check.id}`}>
          <Icon className="h-3.5 w-3.5" />
          {label}
        </Badge>
      </div>
      <p className="mt-1.5 text-sm text-muted">{check.finding}</p>

      {check.couldNotRunReason && (
        <p
          className="mt-2 rounded-md border border-border bg-surface-muted px-3 py-2 text-xs text-muted"
          data-testid={`reason-${check.id}`}
        >
          <span className="font-semibold text-onSurface">Why it could not run: </span>
          {check.couldNotRunReason}
        </p>
      )}

      {check.evidence.length > 0 && (
        <dl className="mt-3 grid gap-1.5 text-xs sm:grid-cols-[minmax(0,14rem)_1fr]">
          {check.evidence.map((e, i) => (
            <div key={i} className="contents">
              <dt className="text-muted">{e.label}</dt>
              <dd className="break-all font-mono text-onSurface">
                {e.value}
                <span className="ml-2 font-sans text-muted">({e.source})</span>
              </dd>
            </div>
          ))}
        </dl>
      )}
    </div>
  );
}

/** The verdict banner. `null` is its own state and must never render as a refusal. */
function VerdictBanner({ report }: { report: BenchReport }) {
  if (report.verdict === null) {
    return (
      <div
        className="flex items-start gap-3 rounded-lg border border-border bg-surface-muted p-4"
        data-testid="verdict-indeterminate"
      >
        <AlertTriangle className="mt-0.5 h-5 w-5 shrink-0 text-warning" />
        <div>
          <p className="font-semibold text-onSurface">No verdict</p>
          <p className="text-sm text-muted">
            The verifier fails closed and produced no verdict at all. This is NOT a refusal of the
            credential - read the rows below to see which checks did and did not run.
            {report.verifierError ? ` Reported: ${report.verifierError}` : ""}
          </p>
        </div>
      </div>
    );
  }
  const ok = report.verdict;
  return (
    <div
      className={`flex items-start gap-3 rounded-lg border p-4 ${
        ok ? "border-success/40 bg-success/10" : "border-danger/40 bg-danger/10"
      }`}
      data-testid="verdict"
    >
      {ok ? (
        <CheckCircle2 className="mt-0.5 h-5 w-5 shrink-0 text-success" />
      ) : (
        <XCircle className="mt-0.5 h-5 w-5 shrink-0 text-danger" />
      )}
      <div>
        <p className="font-semibold text-onSurface">{ok ? "Verifier verdict: valid" : "Verifier verdict: not valid"}</p>
        <p className="text-sm text-muted">
          Copied verbatim from the same verification path the wallet and the verify panel use. The rows
          below are that path&apos;s own reasoning, not a second opinion.
        </p>
      </div>
    </div>
  );
}

function MutationCard({
  mutation,
  onApply,
  disabled,
}: {
  mutation: BenchMutation;
  onApply: (m: BenchMutation) => void;
  disabled: boolean;
}) {
  return (
    <div className="rounded-lg border border-border p-4" data-testid={`mutation-${mutation.id}`}>
      <div className="flex flex-wrap items-start justify-between gap-2">
        <div>
          <p className="text-sm font-medium text-onSurface">{mutation.label}</p>
          <p className="mt-0.5 text-xs text-muted">{mutation.tells}</p>
        </div>
        <Button size="sm" variant="outline" disabled={disabled} onClick={() => onApply(mutation)}>
          Apply &amp; re-run
        </Button>
      </div>

      <p className="mt-3 text-xs text-muted">
        <span className="font-semibold text-onSurface">Designed to be caught by: </span>
        {mutation.caughtBy.length > 0 ? (
          mutation.caughtBy.join(", ")
        ) : (
          // An empty list is a real answer, and the most important one on the page.
          <span className="text-warning" data-testid={`nothing-catches-${mutation.id}`}>
            nothing on this verification path
          </span>
        )}
      </p>

      <details className="mt-2">
        <summary className="cursor-pointer text-xs text-muted">
          What will NOT catch this ({mutation.notCaughtBy.length})
        </summary>
        <ul className="mt-2 space-y-1.5 text-xs text-muted">
          {mutation.notCaughtBy.map((u) => (
            <li key={u.id}>
              <span className="font-mono text-onSurface">{u.id}</span> - {u.why}
            </li>
          ))}
        </ul>
        {mutation.caughtElsewhere && (
          <p className="mt-2 text-xs text-muted">
            <span className="font-semibold text-onSurface">Caught elsewhere: </span>
            {mutation.caughtElsewhere}
          </p>
        )}
      </details>
    </div>
  );
}

export function VerificationBench() {
  const { toast } = useToast();
  const fileRef = useRef<HTMLInputElement>(null);
  const [raw, setRaw] = useState("");
  const [shareUrl, setShareUrl] = useState("");
  const [report, setReport] = useState<BenchReport | null>(null);
  const [applied, setApplied] = useState<string[]>([]);
  const [busy, setBusy] = useState(false);

  async function runOn(docText: string, note?: string) {
    let parsed: Record<string, unknown>;
    try {
      parsed = JSON.parse(docText);
    } catch (e) {
      toast({ title: "Not valid JSON", description: String(e), variant: "danger" });
      return;
    }
    setBusy(true);
    try {
      // The head is read separately and ONLY to pin the reads. If it cannot be read the run proceeds
      // unpinned and says so - it never stamps the rows with a height they were not read at.
      let blockNumber: bigint | undefined;
      try {
        const { roaxPublicClient } = await import("@dogtag/ui");
        blockNumber = await roaxPublicClient(env.roaxRpc).getBlockNumber();
      } catch {
        blockNumber = undefined;
      }
      const next = await runVerificationBench({
        wrappedDoc: parsed,
        rpcUrl: env.roaxRpc,
        registryAddr: env.issuerRegistryAddr || undefined,
        factoryAddr: env.factoryAddr || undefined,
        domainRegistryAddr: env.issuerDomainRegistryAddr || undefined,
        blockNumber,
      });
      setReport(next);
      if (note) setApplied((a) => [...a, note]);
    } finally {
      setBusy(false);
    }
  }

  function onFile(e: ChangeEvent<HTMLInputElement>) {
    const f = e.target.files?.[0];
    if (!f) return;
    f.text().then((t) => {
      setRaw(t);
      void runOn(t);
    });
    e.target.value = "";
  }

  async function fetchShareLink() {
    const url = shareUrl.trim();
    if (!url) return;
    setBusy(true);
    try {
      const res = await fetch(url, { headers: { accept: "application/json" } });
      if (!res.ok) throw new Error(`${res.status} ${res.statusText}`);
      const body = (await res.json()) as Record<string, unknown>;
      // Share endpoints return either the wrapped doc itself or an envelope carrying it.
      const doc = (body.wrappedDoc ?? body.wrapped_doc ?? body) as Record<string, unknown>;
      const text = JSON.stringify(doc, null, 2);
      setRaw(text);
      setApplied([]);
      await runOn(text);
    } catch (e) {
      toast({
        title: "Could not fetch that link",
        description: e instanceof Error ? e.message : String(e),
        variant: "danger",
      });
    } finally {
      setBusy(false);
    }
  }

  function applyMutation(m: BenchMutation) {
    let parsed: Record<string, unknown>;
    try {
      parsed = JSON.parse(raw);
    } catch {
      toast({ title: "Load a record first", variant: "danger" });
      return;
    }
    const out = m.apply(parsed);
    if (!out) {
      // Refusing is the honest outcome; a button that silently did nothing would read as "applied".
      toast({
        title: "Not applicable to this record",
        description: `${m.label} needs a field this document does not carry, so nothing was changed.`,
        variant: "danger",
      });
      return;
    }
    const text = JSON.stringify(out.doc, null, 2);
    setRaw(text);
    void runOn(text, out.note);
  }

  return (
    <div className="space-y-6">
      <Card>
        <CardHeader>
          <CardTitle className="flex items-center gap-2">
            <FlaskConical className="h-5 w-5" />
            Load a record
          </CardTitle>
          <CardDescription>
            Paste a wrapped credential, upload one, or fetch the share link a credential QR encodes.
            Every check below runs against the real verification path; nothing is simulated.
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-4">
          <div>
            <Label htmlFor="bench-doc">Wrapped document JSON</Label>
            <textarea
              id="bench-doc"
              data-testid="bench-doc"
              value={raw}
              onChange={(e) => {
                setRaw(e.target.value);
                setApplied([]);
              }}
              rows={10}
              spellCheck={false}
              className="mt-1 w-full rounded-md border border-border bg-surface px-3 py-2 font-mono text-xs text-onSurface"
              placeholder='{"data":{...},"signature":{...},"issuer":{...}}'
            />
          </div>

          <div className="flex flex-wrap items-end gap-3">
            <Button onClick={() => void runOn(raw)} disabled={busy || !raw.trim()} data-testid="run-bench">
              {busy ? "Running..." : "Run the checks"}
            </Button>
            <Button variant="outline" onClick={() => fileRef.current?.click()} disabled={busy}>
              <Upload className="h-4 w-4" />
              Upload .json
            </Button>
            <input ref={fileRef} type="file" accept=".json,application/json" hidden onChange={onFile} />
            <div className="min-w-[18rem] flex-1">
              <Label htmlFor="bench-qr">QR share link</Label>
              <div className="mt-1 flex gap-2">
                <Input
                  id="bench-qr"
                  data-testid="bench-qr"
                  value={shareUrl}
                  onChange={(e) => setShareUrl(e.target.value)}
                  placeholder="https://.../r/<token>"
                />
                <Button variant="outline" onClick={() => void fetchShareLink()} disabled={busy || !shareUrl.trim()}>
                  Fetch
                </Button>
              </div>
            </div>
          </div>
          <p className="text-xs text-muted">
            A DogTag credential QR encodes a share URL, so pasting that URL is the QR path. Scanning
            with a camera is not implemented here - no scanner primitive exists in this codebase yet.
          </p>
        </CardContent>
      </Card>

      {report && (
        <>
          <VerdictBanner report={report} />

          <Card>
            <CardHeader>
              <CardTitle>Checks</CardTitle>
              <CardDescription>
                One row per check.{" "}
                {report.blockNumber === null
                  ? "The chain head could not be read, so the on-chain reads used `latest` and this report is NOT reproducible."
                  : `Every on-chain read was pinned to block ${report.blockNumber.toString()}.`}
              </CardDescription>
            </CardHeader>
            <CardContent className="pt-0">
              {report.checks.map((c) => (
                <CheckRow key={c.id} check={c} />
              ))}
            </CardContent>
          </Card>

          <Card>
            <CardHeader>
              <CardTitle>Reads made</CardTitle>
              <CardDescription>
                Every chain read the verifier actually performed, as observed. This is the raw material
                behind the evidence above.
              </CardDescription>
            </CardHeader>
            <CardContent className="pt-0">
              <div className="overflow-x-auto">
                <table className="w-full text-xs">
                  <thead className="text-left text-muted">
                    <tr>
                      <th className="py-1 pr-3 font-medium">Method</th>
                      <th className="py-1 pr-3 font-medium">Contract</th>
                      <th className="py-1 pr-3 font-medium">Result</th>
                    </tr>
                  </thead>
                  <tbody className="font-mono">
                    {report.reads.map((r, i) => (
                      <tr key={i} className="border-t border-border">
                        <td className="py-1 pr-3">{r.method}</td>
                        <td className="break-all py-1 pr-3">{r.contract}</td>
                        <td className="break-all py-1 pr-3">
                          {r.outcome === "ok" ? (
                            r.value
                          ) : (
                            <span className="text-danger">failed: {r.error}</span>
                          )}
                        </td>
                      </tr>
                    ))}
                    {report.reads.length === 0 && (
                      <tr>
                        <td colSpan={3} className="py-2 text-muted">
                          No chain read was attempted.
                        </td>
                      </tr>
                    )}
                  </tbody>
                </table>
              </div>
            </CardContent>
          </Card>
        </>
      )}

      <Card>
        <CardHeader>
          <CardTitle>Try to slip a fraudulent record past the checks</CardTitle>
          <CardDescription>
            Each button tells one specific lie with the record above and re-runs every check. Read the
            &ldquo;what will NOT catch this&rdquo; list as carefully as the result - it is the shape of
            the guarantee.
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-3">
          {applied.length > 0 && (
            <div className="rounded-md border border-warning/40 bg-warning/10 px-3 py-2 text-xs" data-testid="applied-mutations">
              <span className="font-semibold text-onSurface">Applied to the loaded record: </span>
              <span className="font-mono">{applied.join(" | ")}</span>
            </div>
          )}
          {BENCH_MUTATIONS.map((m) => (
            <MutationCard key={m.id} mutation={m} onApply={applyMutation} disabled={busy || !raw.trim()} />
          ))}
        </CardContent>
      </Card>
    </div>
  );
}
