import { useEffect, useMemo, useState } from "react";
import { Link, useParams } from "react-router-dom";
import { summarize } from "../lib/credential";
import { integrityOf, redact, shareableFields } from "../lib/redact";
import { useCredentials } from "../lib/hooks";
import { categoryIcon, categoryVar } from "../lib/ui";

const WITHHELD_PLACEHOLDER = "— withheld by holder —";

/**
 * Share a redacted copy — holder-controlled selective disclosure. The holder chooses which fields to
 * reveal; the rest are cryptographically withheld (their cleartext dropped, their leaf hash kept) while
 * the on-chain Merkle root stays the same. The recipient can still confirm the copy is the same,
 * authentic, unmodified credential — they just cannot read the withheld values. This is the Merkle
 * selective-disclosure counterpart to the ZK Present flow, and mirrors the native apps' "Share redacted".
 */
export function Share() {
  const { id = "" } = useParams();
  const decodedId = decodeURIComponent(id);
  const credentials = useCredentials();
  const credential = credentials.find((c) => c.id === decodedId);
  const doc = credential?.wrappedDoc;

  const fields = useMemo(() => (doc ? shareableFields(doc) : []), [doc]);
  // Default: reveal everything. The holder is actively sharing this credential and sees every field —
  // they explicitly choose what to withhold, so nothing the recipient needs is redacted by surprise.
  const [withheld, setWithheld] = useState<Set<string>>(() => new Set());

  const redacted = useMemo(() => (doc ? redact(doc, withheld) : null), [doc, withheld]);
  const redactedJson = useMemo(() => (redacted ? JSON.stringify(redacted, null, 2) : ""), [redacted]);
  const previewIntegrity = useMemo(() => (redacted ? integrityOf(redacted) : "INVALID"), [redacted]);

  const [copyState, setCopyState] = useState<"idle" | "copied" | "failed">("idle");
  const [showJson, setShowJson] = useState(false);

  // Navigating between credentials keeps this component mounted (only the :id param changes), so reset
  // the holder's choices when the credential changes - otherwise A's withheld paths leak into B.
  useEffect(() => {
    setWithheld(new Set());
    setCopyState("idle");
    setShowJson(false);
  }, [decodedId]);

  if (!credential || !doc) {
    return (
      <div className="card">
        <h2>Credential not found</h2>
        <p className="sub">It may have been removed from this wallet.</p>
        <Link to="/wallet" className="btn">
          Back to wallet
        </Link>
      </div>
    );
  }

  const s = summarize(doc);
  const withheldCount = withheld.size;
  const revealableCount = fields.filter((f) => !f.locked).length;

  function toggle(keyPath: string) {
    setCopyState("idle");
    setWithheld((prev) => {
      const next = new Set(prev);
      if (next.has(keyPath)) next.delete(keyPath);
      else next.add(keyPath);
      return next;
    });
  }

  function revealAll() {
    setCopyState("idle");
    setWithheld(new Set());
  }

  function withholdAll() {
    setCopyState("idle");
    setWithheld(new Set(fields.filter((f) => !f.locked).map((f) => f.keyPath)));
  }

  async function copy() {
    try {
      await navigator.clipboard.writeText(redactedJson);
      setCopyState("copied");
    } catch {
      // Clipboard blocked (insecure context / no permission) - do not claim success. Reveal the JSON
      // fallback so the holder can select and copy it manually.
      setCopyState("failed");
      setShowJson(true);
    }
  }

  function download() {
    const blob = new Blob([redactedJson], { type: "application/json" });
    const url = URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = url;
    a.download = `dogtag-${(s.petName ?? s.recordType).replace(/\s+/g, "-").toLowerCase()}-redacted.json`;
    a.click();
    URL.revokeObjectURL(url);
  }

  return (
    <div style={{ ["--cat" as string]: categoryVar(s.category) }}>
      <Link to={`/credential/${encodeURIComponent(credential.id)}`} className="back-link">
        ← {s.petName ?? s.recordType}
      </Link>

      <div className="hero">
        <div className="cred-avatar" aria-hidden>
          {categoryIcon(s.category)}
        </div>
        <div>
          <h1>Share a redacted copy</h1>
          <div className="cred-type">
            {s.recordType} · issued by {s.issuerName}
          </div>
        </div>
      </div>

      <div className="card">
        <h2>Choose what to reveal</h2>
        <p className="sub">
          Withheld fields are cryptographically redacted — the recipient can still verify this is a
          genuine, unmodified credential and check it on-chain, but cannot see the withheld values.
          Your dog tag ID always stays, so the copy remains the same credential.
        </p>

        <div className="btn-row" style={{ marginTop: 0, marginBottom: 4 }}>
          <button
            type="button"
            className="btn secondary"
            data-testid="share-reveal-all"
            onClick={revealAll}
            disabled={withheldCount === 0}
          >
            Reveal all
          </button>
          <button
            type="button"
            className="btn secondary"
            data-testid="share-withhold-all"
            onClick={withholdAll}
            disabled={withheldCount === revealableCount}
          >
            Withhold personal fields
          </button>
        </div>

        <div className="share-list" data-testid="share-fields">
          {fields.map((f) => {
            const isWithheld = withheld.has(f.keyPath);
            return (
              <label className={`share-row ${isWithheld ? "off" : ""}`} key={f.keyPath}>
                <div className="share-field">
                  <div className="k">
                    {f.label}
                    {f.locked && (
                      <span className="lock-tag" data-testid="share-locked">
                        {f.lockReason === "public" ? "🌐 public" : "🔒 required"}
                      </span>
                    )}
                  </div>
                  <div className={`v ${isWithheld ? "withheld" : ""}`}>
                    {isWithheld ? WITHHELD_PLACEHOLDER : f.value}
                  </div>
                </div>
                <input
                  type="checkbox"
                  className="share-toggle"
                  data-testid={`share-toggle-${f.keyPath}`}
                  aria-label={`Reveal ${f.label}`}
                  checked={!isWithheld}
                  disabled={f.locked}
                  onChange={() => toggle(f.keyPath)}
                />
              </label>
            );
          })}
        </div>
      </div>

      <div className="card">
        <div className="section-title" style={{ margin: "0 0 12px" }}>
          <h2 style={{ margin: 0 }}>What the recipient sees</h2>
          <span
            className={`badge ${previewIntegrity === "VALID" ? "ok" : "bad"}`}
            data-testid="share-preview-integrity"
          >
            {previewIntegrity === "VALID" ? "✓ Verifies authentic" : "✗ Integrity broken"}
          </span>
        </div>
        <p className="sub" data-testid="share-withheld-count">
          {withheldCount === 0
            ? "Every field is revealed — this shares the full credential."
            : `${withheldCount} field${withheldCount === 1 ? "" : "s"} withheld from the recipient.`}
        </p>

        <div className="field-grid" data-testid="share-preview">
          {fields.map((f) => {
            const isWithheld = withheld.has(f.keyPath);
            return (
              <div className="field-cell" key={f.keyPath}>
                <div className="k">{f.label}</div>
                <div className={`v ${isWithheld ? "withheld" : ""}`}>
                  {isWithheld ? WITHHELD_PLACEHOLDER : f.value}
                </div>
              </div>
            );
          })}
        </div>

        <div className="btn-row">
          <button className="btn" data-testid="share-copy" onClick={() => void copy()}>
            {copyState === "copied" ? "✓ Copied" : "Copy redacted credential"}
          </button>
          <button type="button" className="btn secondary" data-testid="share-download" onClick={download}>
            Download .json
          </button>
        </div>
        {copyState === "copied" && (
          <p className="ok-box" data-testid="share-copied">
            Redacted credential copied. Paste it wherever the recipient receives credentials — they will
            verify its authenticity offline and on-chain, seeing only the fields you revealed.
          </p>
        )}
        {copyState === "failed" && (
          <p className="error-box" data-testid="share-copy-failed">
            Couldn't reach the clipboard (it may be blocked in this browser). The redacted document is
            shown below - select it and copy manually, or use Download .json.
          </p>
        )}

        <details open={showJson} onToggle={(e) => setShowJson(e.currentTarget.open)}>
          <summary>Show the redacted document (JSON)</summary>
          <textarea
            readOnly
            className="share-output"
            data-testid="share-output"
            value={redactedJson}
            onFocus={(e) => e.currentTarget.select()}
          />
        </details>
      </div>
    </div>
  );
}
