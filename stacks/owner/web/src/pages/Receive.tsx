import { useState } from "react";
import { useNavigate } from "react-router-dom";
import { checkIntegrity } from "@dogtag/standard";
import { parseWrappedDoc, summarize } from "../lib/credential";
import { credentialStore } from "../lib/store";
import { SAMPLE_WRAPPED_DOC_JSON } from "../lib/sampleDoc";

/**
 * Receive a credential an issuer gave you. Every issuer portal (vet / groomer / government) shows a
 * "Copy wrapped document" button after it issues — paste that JSON here. We verify its cryptographic
 * integrity offline (recompute the Merkle root) BEFORE storing, so a tampered credential is rejected
 * at the door. Storage is idempotent by root: re-adding the same credential just refreshes it.
 */
export function Receive() {
  const [text, setText] = useState("");
  const [error, setError] = useState<string | null>(null);
  const navigate = useNavigate();

  function add() {
    setError(null);
    let doc;
    try {
      doc = parseWrappedDoc(text);
    } catch (e) {
      setError((e as Error).message);
      return;
    }
    if (checkIntegrity(doc).state !== "VALID") {
      setError("This credential fails its integrity check — its contents don't match its Merkle root. Not added.");
      return;
    }
    const summary = summarize(doc);
    credentialStore.add(doc, new Date().toISOString());
    navigate(`/credential/${encodeURIComponent(summary.credentialRoot)}`);
  }

  return (
    <div className="card">
      <h2>Receive a credential</h2>
      <p className="sub">
        Paste the wrapped credential document from a vet, groomer, or government portal. It is checked
        for integrity and then held in your wallet on this device.
      </p>
      <label htmlFor="wrapped">Wrapped credential document (JSON)</label>
      <textarea
        id="wrapped"
        data-testid="receive-input"
        placeholder='Paste the "wrapped document" an issuer gave you…'
        value={text}
        onChange={(e) => setText(e.target.value)}
      />
      {error && (
        <p className="error-box" data-testid="receive-error">
          {error}
        </p>
      )}
      <div className="btn-row">
        <button className="btn" data-testid="receive-add" disabled={!text.trim()} onClick={add}>
          Add to wallet
        </button>
        <button
          type="button"
          className="btn secondary"
          data-testid="receive-sample"
          onClick={() => {
            setError(null);
            setText(SAMPLE_WRAPPED_DOC_JSON);
          }}
        >
          Fill sample
        </button>
      </div>
      <div className="notice">
        Nothing is uploaded. The credential is stored only in this browser, exactly as a phone wallet
        holds it. You choose when to present a zero-knowledge proof of it.
      </div>
    </div>
  );
}
