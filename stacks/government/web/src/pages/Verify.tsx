import {
  Badge,
  Button,
  Card,
  DomainBindingBadge,
  Label,
  QrCode,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
  Spinner,
  displayIssuerName,
  explorerTxUrl,
  type IssuerDomainBinding,
  type IssuerIdentity,
} from "@dogtag/ui";
import { CheckCircle2, History, RefreshCw, ShieldCheck, XCircle } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import {
  apiPost,
  VERIFY_PURPOSES,
  verifyHistory,
  verifySessionStart,
  verifySessionStatus,
  type Health,
  type VerifyHistoryItem,
  type VerifySessionStart,
} from "../lib/api";
import { deadlineFromTtl, mmss, useCountdown } from "../lib/countdown";

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

/** Link 1 as a verdict pillar, rendered TRI-state.
 *
 *  Only `notFactoryDeployed` — the factory was asked and answered no — is a definite negative and is the
 *  only value that fails the verdict. `unknown` (no factory configured, or the read failed) is evidence
 *  of nothing and stays neutral, exactly as `couldNotCheck` never renders as `notListed`; colouring it
 *  red would be a lie of emphasis, and would also cry wolf on every credential in a deployment that has
 *  no `FACTORY_ADDR`. */
function ProvenanceFrag({ v }: { v: unknown }) {
  if (v === "factoryDeployed")
    return (
      <Badge data-testid="pillar-provenance" variant="success">
        factory-deployed: yes
      </Badge>
    );
  if (v === "notFactoryDeployed")
    return (
      <Badge data-testid="pillar-provenance" variant="danger">
        factory-deployed: no
      </Badge>
    );
  return (
    <Badge data-testid="pillar-provenance" variant="neutral">
      factory-deployed: not checked
    </Badge>
  );
}

/** How `/v1/verify` chose the contract it checked against — see the `issuerResolution` block. */
interface IssuerResolution {
  source?: "operatorOverride" | "rootIssuer" | "documentClaim";
  rootIssuer?: string | null;
  documentDocumentStore?: string;
  documentStoreDiffers?: boolean;
}

type Phase = "idle" | "starting" | "awaiting" | "verified" | "error" | "failed";

/** A started verify session and the moment its export token lapses, held as ONE value so a session is
 *  never carried without the countdown that governs it. */
interface ActiveSession {
  started: VerifySessionStart;
  deadline: number;
}

/**
 * The owner-hidden verification flow — the authority as a VERIFIER.
 *
 * The government gets no shortcut here: it goes through the same zero-knowledge consent path as any
 * other verifier, gated by the same `VERIFY:<purpose>` whitelist and recorded on the same registry.
 * The owner approves the proof on their own device; nothing on this path reveals who they are or what
 * their credential says.
 */
function OwnerHiddenVerifyFlow({ health }: { health: Health | null }) {
  const [choice, setChoice] = useState<string>(
    `${VERIFY_PURPOSES[0].value}|${VERIFY_PURPOSES[0].recordType}`,
  );
  const [phase, setPhase] = useState<Phase>("idle");
  const [session, setSession] = useState<ActiveSession | null>(null);
  const [txHash, setTxHash] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const timer = useRef<ReturnType<typeof setInterval> | null>(null);
  const remaining = useCountdown(phase === "awaiting" ? (session?.deadline ?? null) : null);
  const selected = VERIFY_PURPOSES.find((p) => `${p.value}|${p.recordType}` === choice);

  const stopPolling = useCallback(() => {
    if (timer.current) clearInterval(timer.current);
    timer.current = null;
  }, []);
  useEffect(() => () => stopPolling(), [stopPolling]);

  function reset() {
    stopPolling();
    setPhase("idle");
    setSession(null);
    setTxHash(null);
    setError(null);
  }

  function beginPolling(sessionId: string) {
    stopPolling();
    timer.current = setInterval(async () => {
      try {
        const s = await verifySessionStatus(sessionId);
        if (s.status === "error") {
          setError(s.txHash || "Verification failed.");
          setPhase("failed");
          stopPolling();
        } else if (s.status === "recorded") {
          setTxHash(s.txHash ?? null);
          setPhase("verified");
          stopPolling();
        }
      } catch {
        // A transient poll failure is not a failed proof; keep polling the durable session.
      }
    }, 3000);
  }

  async function start() {
    if (!selected) return;
    setPhase("starting");
    setError(null);
    setTxHash(null);
    setSession(null);
    try {
      const started = await verifySessionStart(selected.value, selected.recordType);
      setSession({ started, deadline: deadlineFromTtl(started.ttlSecs) });
      setPhase("awaiting");
      beginPolling(started.sessionId);
    } catch (cause) {
      setError((cause as Error).message);
      setPhase("error");
    }
  }

  // The backend refuses honestly rather than failing opaquely when this authority's signer has not been
  // granted the VERIFY namespace on-chain. Surface what to do about it.
  const notWhitelisted = error?.includes("not whitelisted") ?? false;
  const noSigner = error?.includes("no government signer") ?? false;

  return (
    <Card className="p-6">
      <h2 className="flex items-center gap-2 text-base font-semibold text-onSurface">
        <ShieldCheck className="h-5 w-5 text-primary" /> Request owner consent (QR)
      </h2>
      <p className="mt-1 max-w-2xl text-sm text-muted">
        Start a session, show the QR, and let the owner approve a zero-knowledge consent proof on their
        own phone. The authority records the verification on-chain without learning who the owner is or
        what the credential says — the same owner-hidden path every other verifier uses.
      </p>

      {(phase === "idle" || phase === "starting" || phase === "error") && (
        <div className="mt-4 space-y-4">
          <div className="space-y-2">
            <Label htmlFor="verify-purpose">Purpose</Label>
            <Select value={choice} onValueChange={setChoice}>
              <SelectTrigger id="verify-purpose" data-testid="verify-purpose">
                <SelectValue placeholder="Choose a purpose" />
              </SelectTrigger>
              <SelectContent>
                {VERIFY_PURPOSES.map((p) => (
                  <SelectItem key={`${p.value}|${p.recordType}`} value={`${p.value}|${p.recordType}`}>
                    {p.label}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
            {selected && (
              <p className="text-xs text-muted">
                Whitelist namespace{" "}
                <code className="receipt-mono">VERIFY:{selected.value}</code> · record type{" "}
                <code className="receipt-mono">{selected.recordType}</code>
              </p>
            )}
          </div>

          {error && (
            <div
              data-testid="verify-session-error"
              className="rounded-md border border-warning/40 bg-warning/10 p-3 text-sm text-onSurface"
            >
              <strong>{error}</strong>
              {notWhitelisted && selected && (
                <p className="mt-1 text-xs text-muted">
                  This authority&apos;s signer{" "}
                  <code className="receipt-mono">{health?.signer ?? "(none)"}</code> has not been
                  granted the <code className="receipt-mono">VERIFY:{selected.value}</code> namespace on
                  the IssuerRegistry{" "}
                  <code className="receipt-mono">{health?.issuerRegistry ?? "?"}</code>. Verify
                  capability is granted on-chain through the admin apply→approve flow, separately from
                  the issuer roles this authority already holds — until then no owner-hidden
                  verification can be recorded, and the paste-JSON check below still works.
                </p>
              )}
              {noSigner && (
                <p className="mt-1 text-xs text-muted">
                  No <code className="receipt-mono">GOV_SIGNER_KEY</code> is configured, so there is no
                  relayer to bind a consent proof to.
                </p>
              )}
            </div>
          )}

          <Button
            data-testid="verify-session-start"
            loading={phase === "starting"}
            disabled={!selected || phase === "starting"}
            onClick={() => void start()}
          >
            Start verification
          </Button>
        </div>
      )}

      {phase === "awaiting" && session && (
        <div className="mt-4 flex flex-col items-center gap-4">
          <Badge variant="warning">
            <Spinner className="h-3 w-3" /> Awaiting owner consent
          </Badge>
          {remaining > 0 ? (
            <>
              <QrCode value={session.started.qrUrl} caption={session.started.qrUrl} />
              <Badge
                variant={remaining <= 60 ? "warning" : "neutral"}
                data-testid="verify-qr-countdown"
              >
                expires in {mmss(remaining)}
              </Badge>
              <p className="max-w-sm text-center text-sm text-muted">
                Ask the owner to scan this and approve the private proof on their device. Proving takes
                a few tens of seconds on the phone.
              </p>
            </>
          ) : (
            <div className="flex flex-col items-center gap-3 py-4 text-center" data-testid="verify-qr-expired">
              <Badge variant="danger">QR expired</Badge>
              <p className="max-w-sm text-sm text-muted">
                The one-time code lapsed before the owner approved it. Start a fresh session to try
                again.
              </p>
            </div>
          )}
          <Button variant="ghost" size="sm" onClick={reset}>
            {remaining > 0 ? "Cancel" : "Start over"}
          </Button>
        </div>
      )}

      {phase === "verified" && (
        <div className="mt-4 flex flex-col items-center gap-3 py-4 text-center" data-testid="verify-recorded">
          <CheckCircle2 className="h-10 w-10 text-success" />
          <p className="text-lg font-semibold text-onSurface">Verified</p>
          <p className="text-sm text-muted">The owner-hidden consent proof was recorded on ROAX.</p>
          {txHash && (
            <a
              href={explorerTxUrl(txHash)}
              target="_blank"
              rel="noreferrer"
              className="break-all font-mono text-sm text-primary hover:underline"
            >
              {txHash}
            </a>
          )}
          <Button variant="outline" size="sm" onClick={reset}>
            New verification
          </Button>
        </div>
      )}

      {phase === "failed" && (
        <div className="mt-4 flex flex-col items-center gap-3 py-4 text-center">
          <XCircle className="h-10 w-10 text-danger" />
          <p className="text-lg font-semibold text-onSurface">Verification failed</p>
          <p className="break-words text-sm text-danger">
            {error ?? "The proof could not be recorded."}
          </p>
          <Button variant="outline" size="sm" onClick={reset}>
            Try again
          </Button>
        </div>
      )}
    </Card>
  );
}

function sessionVariant(status: string): "success" | "warning" | "danger" | "neutral" {
  if (status === "recorded") return "success";
  if (status === "error") return "danger";
  if (status === "recording" || status === "pending") return "warning";
  return "neutral";
}

/** Owner-hidden verification audit log — proof metadata only, never owner identity. */
function OwnerHiddenHistory() {
  const [items, setItems] = useState<VerifyHistoryItem[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      setItems(await verifyHistory());
    } catch (cause) {
      setError((cause as Error).message);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  return (
    <Card className="p-6">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div>
          <h2 className="flex items-center gap-2 text-base font-semibold text-onSurface">
            <History className="h-5 w-5 text-primary" /> Owner-hidden verification history
          </h2>
          <p className="mt-1 text-sm text-muted">
            Consent verifications this authority recorded. Purpose, relayer and nullifier only — no
            owner identity and no credential contents, on this page or on chain.
          </p>
        </div>
        <Button variant="outline" size="sm" loading={loading} onClick={() => void load()}>
          <RefreshCw className="h-4 w-4" /> Refresh
        </Button>
      </div>

      {error && (
        <p className="mt-4">
          <Badge variant="danger">{error}</Badge>
        </p>
      )}

      <div className="mt-4 space-y-3">
        {items.length === 0 && !loading ? (
          <p className="rounded-md border border-dashed border-border p-4 text-sm text-muted">
            No owner-hidden verifications recorded yet.
          </p>
        ) : (
          items.map((item) => (
            <div
              key={item.sessionId}
              data-testid="verify-history-row"
              className="grid min-w-0 gap-2 rounded-md border border-border p-3 sm:grid-cols-[minmax(0,1fr)_auto]"
            >
              <div className="min-w-0">
                <p className="font-medium text-onSurface">{item.purpose}</p>
                <p className="text-xs text-muted">
                  {item.recordType} ·{" "}
                  {new Date((item.updatedAt || item.createdAt) * 1000).toLocaleString()}
                </p>
                {item.nullifier && (
                  <p className="receipt-mono mt-1 truncate text-xs text-muted" title={item.nullifier}>
                    nullifier {item.nullifier}
                  </p>
                )}
              </div>
              <div className="flex flex-wrap items-center gap-2 sm:justify-end">
                <Badge variant={sessionVariant(item.status)}>{item.status}</Badge>
                {item.txHash?.startsWith("0x") && (
                  <a
                    href={item.explorerUrl || explorerTxUrl(item.txHash)}
                    target="_blank"
                    rel="noreferrer"
                    className="max-w-48 truncate font-mono text-xs text-primary hover:underline"
                    title={item.txHash}
                  >
                    {item.txHash}
                  </a>
                )}
              </div>
            </div>
          ))
        )}
      </div>
    </Card>
  );
}

/** The paste-a-wrappedDoc check: offline integrity + on-chain status + issuer identity.
 *
 *  Kept alongside the QR flow deliberately — it needs no phone, no owner present and no consent, so it
 *  stays the right tool for checking a document someone emailed over or an officer already holds. */
function PasteDocVerify() {
  const [doc, setDoc] = useState("");
  const [signer, setSigner] = useState("");
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<Record<string, unknown> | null>(null);
  const [error, setError] = useState<string | null>(null);

  // Deliberately NOT prefilled with this authority's own signer. The whitelist pillar resolves its
  // own signer from the chain now, so this field is only an EXPECTED-signer assertion that tightens
  // the pillar - and prefilling it would silently assert "this credential must have been issued by
  // US", failing the documented cross-role flow where a vet issues and the government verifies.
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
  const identity = result?.issuerIdentity as IssuerIdentity | undefined;
  const binding = result?.issuerDomainBinding as IssuerDomainBinding | undefined;
  const resolution = result?.issuerResolution as IssuerResolution | undefined;

  return (
    <Card className="p-6">
      <h2 className="text-base font-semibold text-onSurface">Verify a document (paste JSON)</h2>
      <p className="mt-1 max-w-2xl text-sm text-muted">
        Recomputes integrity (offline) and reads on-chain status (DogTagIssuer.isValid) + issuer
        identity (IssuerRegistry.isWhitelistedFor) from ROAX. All reads are gasless. No owner and no
        phone needed — this checks a document, not a consent.
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
        Expected issuer signer (optional)
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
            <ProvenanceFrag v={frag?.issuerProvenance} />
          </div>

          <IssuerLine identity={identity} binding={binding} resolution={resolution} />
          <pre className="receipt-mono mt-4 max-h-80 overflow-auto rounded-md border border-border bg-surface-muted p-3 text-xs text-onSurface">
            {JSON.stringify(result, null, 2)}
          </pre>
        </div>
      )}
    </Card>
  );
}

/** The issuer, as it may honestly be shown: the ON-CHAIN name, with the domain binding beside it.
 *
 *  The credential's own `issuer` block is outside the Merkle root, so it is never rendered as the
 *  issuer — only as a stated disagreement when it contradicts the chain. That is the whole fix for the
 *  audit's relabelling demo: a re-badged document now displays the authority that actually issued it. */
function IssuerLine({
  identity,
  binding,
  resolution,
}: {
  identity: IssuerIdentity | undefined;
  binding: IssuerDomainBinding | undefined;
  resolution: IssuerResolution | undefined;
}) {
  if (!identity && !binding) return null;
  const { name, sourceLabel } = displayIssuerName(identity);

  return (
    <div data-testid="issuer-line" className="mt-4 border-t border-border pt-3">
      <div className="text-xs font-medium text-muted">Issuer</div>
      <div className="mt-1 flex flex-wrap items-center gap-x-2 gap-y-1">
        <span data-testid="issuer-name" className="text-sm font-medium text-onSurface">
          {name}
        </span>
        {/* Never let a fallback pass for the authoritative value — and say which of the two withheld
            cases actually happened, since "not factory-descended" and "could not be read" are
            different facts. The copy lives in `displayIssuerName` so surfaces cannot drift. */}
        <span data-testid="issuer-name-source" className="text-[11px] text-muted">
          {sourceLabel}
        </span>
      </div>

      {/* The badge: small, beside the issuer, an observation rather than a verdict. */}
      <div className="mt-1.5">
        {/* This is an audit surface, so the block anchor and the live-vs-recorded DNS label are shown:
            "verified" without a "when" is not auditable against a world where DNS changes. */}
        <DomainBindingBadge binding={binding} showProvenance data-testid="issuer-domain-binding" />
      </div>

      {/* Stated only when the document contradicts the chain. Factual, no alarm language: the
          credential's validity is reported separately and is not what this is about. */}
      {identity?.documentNameDiffers && (
        <p data-testid="issuer-name-differs" className="mt-1.5 text-xs text-warning">
          The document names a different issuer: “{identity.documentName}”
        </p>
      )}
      {/* The chain says a different contract issued this root than the document claims. Reported, never
          silently followed — following the document's claim is how a swapped documentStore redirects a
          check at a contract the attacker controls. */}
      {resolution?.documentStoreDiffers && (
        <p data-testid="issuer-store-differs" className="mt-1.5 text-xs text-warning">
          The document names contract {resolution.documentDocumentStore}, but the chain records{" "}
          {resolution.rootIssuer} as the issuer of this credential
        </p>
      )}
      {identity?.documentDomainDiffers && (
        <p data-testid="issuer-domain-differs" className="mt-1 text-xs text-warning">
          The document claims the domain “{identity.documentDomain}”, but this credential was issued
          under “{identity.rootCoveredDomain}”
        </p>
      )}
      {identity?.assertion === "notAssertable" && (
        <p data-testid="issuer-did-not-assertable" className="mt-1 text-xs text-muted">
          This document carries no issuer identity inside its Merkle root, so its issuer domain could
          not be cross-checked.
        </p>
      )}
    </div>
  );
}

export function Verify({ health }: { health: Health | null }) {
  return (
    <div className="space-y-4">
      <OwnerHiddenVerifyFlow health={health} />
      <PasteDocVerify />
      <OwnerHiddenHistory />
    </div>
  );
}
