import { useMemo, useState } from "react";
import { Link, useSearchParams } from "react-router-dom";
import { summarize } from "../lib/credential";
import { useCredentials } from "../lib/hooks";
import { PROVER_URL, explorerTxUrl } from "../lib/config";
import { presentCredential, type PresentProgress, type PresentStep } from "../lib/present";
import type { OwnerWallet } from "../lib/wallet";

const STEP_ORDER: PresentStep[] = ["resolving", "signing", "proving", "binding", "submitting", "polling"];
const STEP_LABEL: Record<PresentStep, string> = {
  resolving: "Read the verifier's request",
  signing: "Sign consent with your on-device key",
  proving: "Generate the zero-knowledge proof",
  binding: "Authorize gasless consent-key bind",
  submitting: "Submit the proof to the verifier",
  polling: "Confirm on-chain",
  verified: "Verified",
  failed: "Failed",
};

type Outcome =
  | { kind: "idle" }
  | { kind: "running"; step: PresentStep; note?: string }
  | { kind: "verified"; txHash: string | null; nullifier: string | null }
  | { kind: "error"; message: string; step: PresentStep };

export function Present({ wallet }: { wallet: OwnerWallet | null }) {
  const credentials = useCredentials();
  const [params] = useSearchParams();
  const preselect = params.get("id") ?? "";

  const [credId, setCredId] = useState(preselect || credentials[0]?.id || "");
  const [link, setLink] = useState("");
  const [proverUrl, setProverUrl] = useState(PROVER_URL);
  const [outcome, setOutcome] = useState<Outcome>({ kind: "idle" });

  const selected = useMemo(() => credentials.find((c) => c.id === credId), [credentials, credId]);
  const running = outcome.kind === "running";

  async function run() {
    if (!wallet || !selected) return;
    setOutcome({ kind: "running", step: "resolving" });
    try {
      const result = await presentCredential({
        link,
        wallet,
        doc: selected.wrappedDoc,
        proverUrl: proverUrl.replace(/\/+$/, ""),
        onProgress: (p: PresentProgress) => {
          if (p.step !== "verified" && p.step !== "failed") {
            setOutcome({ kind: "running", step: p.step, note: p.note });
          }
        },
      });
      setOutcome({ kind: "verified", txHash: result.txHash, nullifier: result.nullifier });
    } catch (e) {
      const step = outcome.kind === "running" ? outcome.step : "resolving";
      setOutcome({ kind: "error", message: (e as Error).message, step });
    }
  }

  if (credentials.length === 0) {
    return (
      <div className="card">
        <h2>Nothing to present</h2>
        <p className="sub">Receive a credential first, then present a zero-knowledge proof of it.</p>
        <Link to="/receive" className="btn">
          Receive a credential
        </Link>
      </div>
    );
  }

  const stepIndex =
    outcome.kind === "running" || outcome.kind === "error" ? STEP_ORDER.indexOf(outcome.step) : -1;

  return (
    <div className="card">
      <h2>Present a proof</h2>
      <p className="sub">
        A verifier (a groomer, border officer, venue…) shows you a QR / link. Presenting proves your
        pet holds a valid credential for their purpose — <strong>without revealing its contents</strong>.
        The proof is generated with your own key; only the result is recorded on-chain.
      </p>

      <label htmlFor="present-cred">Credential to present</label>
      <select
        id="present-cred"
        data-testid="present-credential"
        value={credId}
        onChange={(e) => setCredId(e.target.value)}
        disabled={running}
      >
        {credentials.map((c) => {
          const s = summarize(c.wrappedDoc);
          return (
            <option key={c.id} value={c.id}>
              {(s.petName ? `${s.petName} · ` : "") + s.recordType} — {s.issuerName}
            </option>
          );
        })}
      </select>

      <label htmlFor="present-link">Verifier link</label>
      <input
        id="present-link"
        data-testid="present-link"
        placeholder="https://verifier.example/x/<token>?a=0x…"
        value={link}
        onChange={(e) => setLink(e.target.value)}
        disabled={running}
      />

      <label htmlFor="present-prover">Your trusted prover-service</label>
      <input
        id="present-prover"
        data-testid="present-prover"
        value={proverUrl}
        onChange={(e) => setProverUrl(e.target.value)}
        disabled={running}
      />

      <button
        className="btn block"
        data-testid="present-run"
        disabled={running || !wallet || !selected || !link.trim() || !proverUrl.trim()}
        onClick={() => void run()}
      >
        {running ? "Presenting…" : wallet ? "Present proof" : "Preparing wallet…"}
      </button>

      {(running || outcome.kind === "error") && (
        <div className="steps" data-testid="present-steps">
          {STEP_ORDER.map((st, i) => {
            const done = i < stepIndex;
            const active = running && i === stepIndex;
            const failed = outcome.kind === "error" && i === stepIndex;
            const cls = active ? "active" : done ? "done" : "";
            return (
              <div className={`step ${cls}`} key={st} data-testid={`step-${st}`}>
                <span className="dot">{done ? "✓" : failed ? "✗" : i + 1}</span>
                <span>
                  {STEP_LABEL[st]}
                  {active && outcome.note ? ` — ${outcome.note}` : ""}
                </span>
              </div>
            );
          })}
        </div>
      )}

      {outcome.kind === "verified" && (
        <div className="result ok" data-testid="present-verified">
          <div className="ring" aria-hidden>
            ✓
          </div>
          <h2>Verified</h2>
          <p className="sub" style={{ margin: "0 auto", maxWidth: 360 }}>
            The verifier recorded a valid proof on ROAX. No credential data was disclosed — a genuine
            zero-knowledge presentation.
          </p>
          {outcome.txHash && (
            <a className="txlink" href={explorerTxUrl(outcome.txHash)} target="_blank" rel="noreferrer" data-testid="present-tx">
              {outcome.txHash}
            </a>
          )}
          {outcome.nullifier && (
            <div className="kv" style={{ marginTop: 12, textAlign: "left" }}>
              <span className="k">Nullifier</span>
              <span className="v" data-testid="present-nullifier">
                {outcome.nullifier}
              </span>
            </div>
          )}
          <div className="btn-row" style={{ justifyContent: "center" }}>
            <button type="button" className="btn secondary" onClick={() => setOutcome({ kind: "idle" })}>
              Done
            </button>
          </div>
        </div>
      )}

      {outcome.kind === "error" && (
        <div className="error-box" data-testid="present-error">
          {outcome.message}
        </div>
      )}

      <details>
        <summary>How does this stay private?</summary>
        <pre>
{`1. You sign a one-time consent with your BabyJubjub key (in this browser).
2. Your trusted prover turns the credential + consent into a Groth16 proof.
   The verifier never sees the credential — only the proof.
3. The verifier relays the proof on-chain (it pays gas; you stay gasless).
4. The chain checks the proof, binds your consent key, and records a nullifier
   so the same consent can't be replayed. Nothing about your pet is disclosed.`}
        </pre>
      </details>
    </div>
  );
}
