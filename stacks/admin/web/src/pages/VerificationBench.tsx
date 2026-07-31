import {
  BENCH_MUTATIONS,
  BENCH_SCENARIOS,
  Badge,
  Button,
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
  Input,
  Label,
  docFromShareResponse,
  runBenchScenario,
  runVerificationBench,
  useRoaxRpcPreference,
  useToast,
  type BenchCheck,
  type BenchCheckId,
  type BenchMutation,
  type BenchReport,
  type BenchScenario,
  type CheckOutcome,
} from "@dogtag/ui";
import {
  AlertTriangle,
  CheckCircle2,
  FlaskConical,
  HelpCircle,
  ShieldAlert,
  Upload,
  XCircle,
} from "lucide-react";
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
        <div className="flex items-center gap-2">
          {/* Which rows the verifier's verdict folds in. Without this a red expiry row under a green
              banner reads as a self-contradiction rather than the real property it is. */}
          {!check.gatesVerdict && (
            <span className="text-xs text-muted" data-testid={`beside-${check.id}`}>
              not in the verdict
            </span>
          )}
          <Badge variant={variant} data-testid={`outcome-${check.id}`}>
            <Icon className="h-3.5 w-3.5" />
            {label}
          </Badge>
        </div>
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
          Copied verbatim from the same verification path the wallet and the verify panel use. It covers
          integrity, on-chain status and the issuer pillar - NOT expiry or the issuer domain, which are
          reported below as their own rows. Read both.
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

/** What one scenario actually did, beside what it declared it should do. */
interface ScenarioRun {
  report: BenchReport;
  /** Checks whose real outcome differs from the declared expectation. Empty is the normal case. */
  unexpected: BenchCheckId[];
}

/**
 * One fraudulent record from the catalogue, run against its own scripted chain.
 *
 * The declared expectation and the ACTUAL outcome are rendered side by side, and a mismatch between
 * them is called out rather than smoothed over: an attack catalogue whose expectations were quietly
 * reconciled with whatever the code prints would certify itself.
 */
function ScenarioCard({
  scenario,
  run,
  onRun,
  busy,
}: {
  scenario: BenchScenario;
  run: ScenarioRun | undefined;
  onRun: (s: BenchScenario) => void;
  busy: boolean;
}) {
  const refused = scenario.refusedBy;
  return (
    <div className="rounded-lg border border-border p-4" data-testid={`scenario-${scenario.id}`}>
      <div className="flex flex-wrap items-start justify-between gap-2">
        <div className="max-w-2xl">
          <p className="text-sm font-medium text-onSurface">{scenario.title}</p>
          <p className="mt-0.5 text-xs text-muted">{scenario.tells}</p>
        </div>
        <Button size="sm" variant="outline" disabled={busy} onClick={() => onRun(scenario)}>
          Run it
        </Button>
      </div>

      <p className="mt-3 text-xs text-muted">
        {scenario.mustVerify ? (
          <>
            <span className="font-semibold text-onSurface">Must VERIFY: </span>
            this record is genuine and the verifier is required not to refuse it.
          </>
        ) : (
          <>
            <span className="font-semibold text-onSurface">Must be refused by: </span>
            {refused.length > 0 ? (
              <span className="font-mono">{refused.join(", ")}</span>
            ) : (
              <span className="text-warning">
                no single check - see the note below for what should happen instead
              </span>
            )}
          </>
        )}
      </p>

      {/* The finding, rendered as loudly as it reads. A divergence is the most valuable thing this
          page can tell an operator, so it is never hidden behind a disclosure triangle. */}
      {scenario.divergence && (
        <div
          className="mt-3 flex items-start gap-2 rounded-md border border-danger/40 bg-danger/10 px-3 py-2 text-xs"
          data-testid={`divergence-${scenario.id}`}
        >
          <ShieldAlert className="mt-0.5 h-4 w-4 shrink-0 text-danger" />
          <p className="text-onSurface">
            <span className="font-semibold">Known divergence: </span>
            {scenario.divergence}
          </p>
        </div>
      )}

      {run && (
        <div className="mt-3 space-y-2">
          <div className="flex flex-wrap items-center gap-2 text-xs">
            <span className="text-muted">Actual verdict:</span>
            <Badge
              variant={
                run.report.verdict === null ? "neutral" : run.report.verdict ? "success" : "danger"
              }
              data-testid={`scenario-verdict-${scenario.id}`}
            >
              {run.report.verdict === null ? "no verdict" : run.report.verdict ? "valid" : "not valid"}
            </Badge>
            {run.unexpected.length === 0 ? (
              <span className="text-muted">matched the declared expectation for every check</span>
            ) : (
              <span className="text-danger" data-testid={`scenario-unexpected-${scenario.id}`}>
                DIVERGED from the declared expectation: {run.unexpected.join(", ")}
              </span>
            )}
          </div>
          <div className="flex flex-wrap gap-1.5">
            {run.report.checks.map((c) => (
              <span
                key={c.id}
                className={`rounded px-1.5 py-0.5 font-mono text-[11px] ${
                  c.outcome === "pass"
                    ? "bg-success/15 text-success"
                    : c.outcome === "fail"
                      ? "bg-danger/15 text-danger"
                      : "bg-surface-muted text-muted"
                }`}
                title={c.finding}
              >
                {c.id}: {c.outcome}
                {/* Colour alone cannot say which rows the verdict folds in, and on this card the
                    whole point of some scenarios is that a red GATING row sits beside a green
                    ADVISORY one. Unmarked, that reads as nine-green-one-red rather than as the
                    asymmetry it is. */}
                {!c.gatesVerdict && <span className="ml-0.5 opacity-70">&dagger;</span>}
              </span>
            ))}
          </div>
          <p className="text-[11px] text-muted">
            &dagger; reported beside the verifier&apos;s verdict, not folded into it.
          </p>

          {/* A failing row's FINDING, in the open. It was previously reachable only by hovering the
              chip, which hides the one sentence saying what actually went wrong. */}
          {run.report.checks.some((c) => c.outcome === "fail") && (
            <ul className="space-y-1 text-xs text-muted" data-testid={`findings-${scenario.id}`}>
              {run.report.checks
                .filter((c) => c.outcome === "fail")
                .map((c) => (
                  <li key={c.id}>
                    <span className="font-mono text-danger">{c.id}</span>
                    {!c.gatesVerdict && <span className="text-muted"> (advisory)</span>} -{" "}
                    {c.finding}
                  </li>
                ))}
            </ul>
          )}
        </div>
      )}

      {scenario.blindSpots.length > 0 && (
        <details className="mt-2">
          <summary className="cursor-pointer text-xs text-muted">
            What does NOT object to this ({scenario.blindSpots.length})
          </summary>
          <ul className="mt-2 space-y-1.5 text-xs text-muted">
            {scenario.blindSpots.map((b) => (
              <li key={b.id}>
                <span className="font-mono text-onSurface">{b.id}</span> - {b.why}
              </li>
            ))}
          </ul>
        </details>
      )}

      {scenario.notes && <p className="mt-2 text-xs text-muted">{scenario.notes}</p>}
    </div>
  );
}

export function VerificationBench() {
  const { toast } = useToast();
  const rpc = useRoaxRpcPreference(env.roaxRpc);
  const fileRef = useRef<HTMLInputElement>(null);
  const [raw, setRaw] = useState("");
  const [shareUrl, setShareUrl] = useState("");
  const [report, setReport] = useState<BenchReport | null>(null);
  const [applied, setApplied] = useState<string[]>([]);
  const [busy, setBusy] = useState(false);
  const [runs, setRuns] = useState<Record<string, ScenarioRun>>({});

  /**
   * Run one catalogue scenario. It carries its own scripted chain, so this makes NO network call and
   * cannot be perturbed by whatever the live chain happens to hold - which is the only way a case like
   * "the signer was delisted after it issued" can be exercised at all.
   */
  async function runScenario(s: BenchScenario) {
    setBusy(true);
    try {
      const report = await runBenchScenario(s);
      const unexpected = report.checks
        .filter((c) => s.expected[c.id] !== c.outcome)
        .map((c) => c.id);
      setRuns((prev) => ({ ...prev, [s.id]: { report, unexpected } }));
    } catch (e) {
      toast({
        title: `Scenario "${s.title}" could not run`,
        description: e instanceof Error ? e.message : String(e),
        variant: "danger",
      });
    } finally {
      setBusy(false);
    }
  }

  async function runAllScenarios() {
    setBusy(true);
    try {
      const next: Record<string, ScenarioRun> = {};
      for (const s of BENCH_SCENARIOS) {
        const report = await runBenchScenario(s);
        next[s.id] = {
          report,
          unexpected: report.checks.filter((c) => s.expected[c.id] !== c.outcome).map((c) => c.id),
        };
      }
      setRuns(next);
    } catch (e) {
      toast({
        title: "The catalogue could not run",
        description: e instanceof Error ? e.message : String(e),
        variant: "danger",
      });
    } finally {
      setBusy(false);
    }
  }

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
        blockNumber = await roaxPublicClient(
          rpc.rpcUrl,
          rpc.defaultRpcUrl,
        ).getBlockNumber();
      } catch {
        blockNumber = undefined;
      }
      const next = await runVerificationBench({
        wrappedDoc: parsed,
        rpcUrl: rpc.rpcUrl,
        defaultRpcUrl: rpc.defaultRpcUrl,
        registryAddr: env.issuerRegistryAddr || undefined,
        factoryAddr: env.factoryAddr || undefined,
        domainRegistryAddr: env.issuerDomainRegistryAddr || undefined,
        blockNumber,
      });
      setReport(next);
      if (note) setApplied((a) => [...a, note]);
    } catch (e) {
      // The engine reports a bad record rather than throwing, so reaching here means something else
      // broke. Say so and CLEAR the report: leaving the previous one on screen shows a stale answer as
      // though it applied to the record just submitted, which is this page's cardinal sin.
      setReport(null);
      toast({
        title: "The bench could not run",
        description: e instanceof Error ? e.message : String(e),
        variant: "danger",
      });
    } finally {
      setBusy(false);
    }
  }

  function onFile(e: ChangeEvent<HTMLInputElement>) {
    const f = e.target.files?.[0];
    if (!f) return;
    f.text()
      .then((t) => {
        setRaw(t);
        void runOn(t);
      })
      .catch((err: unknown) => {
        toast({
          title: "Could not read that file",
          description: err instanceof Error ? err.message : String(err),
          variant: "danger",
        });
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
      const doc = docFromShareResponse(body);
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
                  : `Every on-chain read was pinned to block ${report.blockNumber.toString()}.`}{" "}
                Rows marked <span className="font-medium">not in the verdict</span> are reported beside
                it rather than folded into it - the verifier&apos;s verdict is integrity, on-chain
                status and the issuer pillar only. That is why an expired credential can be
                on-chain-valid: the chain records anchoring and revocation, and has no concept of a
                validity window.
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

      <Card>
        <CardHeader>
          <CardTitle>The attack catalogue</CardTitle>
          <CardDescription>
            Complete fraudulent records, each with its own scripted chain, run through the same
            verification path as everything above. Unlike the buttons in the previous card these need no
            loaded record and make no network call: they exercise chain states a live chain cannot be
            asked to produce - a signer delisted after it issued, a contract the factory never
            deployed vouching for a root, a registry that does not govern the clone. Each declares WHICH
            check must refuse it, and a run that diverges from that declaration is called out in red.
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-3">
          <div className="flex flex-wrap items-center gap-3">
            <Button onClick={() => void runAllScenarios()} disabled={busy} data-testid="run-catalogue">
              {busy ? "Running..." : "Run the whole catalogue"}
            </Button>
            <p className="text-xs text-muted">
              {BENCH_SCENARIOS.length} scenarios, including one honest control - without it a
              catalogue of nothing but frauds would look perfect against a verifier that refused
              everything.
            </p>
          </div>
          {BENCH_SCENARIOS.map((s) => (
            <ScenarioCard
              key={s.id}
              scenario={s}
              run={runs[s.id]}
              onRun={(x) => void runScenario(x)}
              busy={busy}
            />
          ))}
        </CardContent>
      </Card>
    </div>
  );
}
