import { Link, useParams } from "react-router-dom";
import { ROAX_CHAIN_ID, ROAX_EXPLORER_URL, VERIFICATION_REGISTRY_CONSENT_ADDR } from "../lib/config";
import { humanizePurpose, windowOpen } from "../lib/consents";
import { useConsentReceipts, useCredentials } from "../lib/hooks";

/**
 * One consent receipt in full - the owner-facing record of a single point-in-time grant: purpose,
 * relayer, record type, consent window, timestamp, and its on-chain confirmation (tx + nullifier).
 * Print is the only export: this is the owner's private audit record, shown only to the owner.
 * There is deliberately nothing to cancel here - the proof was spent when it was recorded.
 */

function fmtDateTime(unixSec: number): string {
  return new Date(unixSec * 1000).toLocaleString();
}

export function ConsentDetail() {
  const { nullifier = "" } = useParams();
  const decodedNullifier = decodeURIComponent(nullifier).toLowerCase();
  const credentials = useCredentials();
  const { state, receipts, reload } = useConsentReceipts(credentials);
  const receipt = receipts.find((r) => r.nullifier === decodedNullifier);

  if (state === "loading") {
    return (
      <div className="card" data-testid="consent-detail-loading">
        <h2>Reading ROAX…</h2>
        <p className="sub">Looking up this consent's on-chain record.</p>
      </div>
    );
  }

  if (state === "error") {
    return (
      <div className="card" data-testid="consent-detail-error">
        <h2>Could not reach ROAX</h2>
        <p className="sub">This consent's record lives on-chain; retry when the network is reachable.</p>
        <div className="btn-row">
          <button type="button" className="btn" onClick={reload}>
            Try again
          </button>
          <Link to="/consents" className="btn secondary">
            Back to consents
          </Link>
        </div>
      </div>
    );
  }

  if (!receipt) {
    return (
      <div className="card" data-testid="consent-not-found">
        <h2>Consent not found</h2>
        <p className="sub">
          No verification with this nullifier exists on your held tags. If you removed the matching
          credential from this wallet, its consent history is no longer attributable here.
        </p>
        <Link to="/consents" className="btn">
          Back to consents
        </Link>
      </div>
    );
  }

  const open = windowOpen(receipt);
  const who = receipt.petName ?? (receipt.handle ? `Tag #${receipt.handle}` : "Unknown tag");

  return (
    <div>
      <div className="no-print receipt-controls">
        <Link to="/consents" className="back-link" style={{ margin: 0 }}>
          ← Back to consents
        </Link>
        <div className="btn-row" style={{ marginTop: 0 }}>
          <button
            type="button"
            className="btn"
            data-testid="consent-print"
            onClick={() => window.print()}
          >
            🖨 Print receipt
          </button>
        </div>
      </div>

      <div className="card" data-testid="consent-detail">
        <div className="cred-meta" style={{ marginTop: 0 }}>
          <span className={`badge ${open ? "ok" : ""}`} data-testid="consent-detail-window">
            {open
              ? `Window open until ${fmtDateTime(receipt.deadline)}`
              : `Window closed (${fmtDateTime(receipt.deadline)})`}
          </span>
          <span className="badge ok" data-testid="consent-detail-onchain">
            ✓ Recorded on-chain
          </span>
        </div>

        <h2 style={{ marginTop: 18 }} data-testid="consent-detail-purpose">
          {receipt.purposeLabel ? humanizePurpose(receipt.purposeLabel) : "Consent"}
        </h2>
        <p className="sub">
          You approved this consent on your device as a zero-knowledge proof. It authorized exactly
          one verification - recorded below, without your identity - and was spent the moment it was
          recorded. This receipt is permanent history.
        </p>

        <h2 style={{ marginTop: 20 }}>What you consented to</h2>
        <div>
          <div className="kv">
            <span className="k">Pet / tag</span>
            <span className="v" data-testid="consent-detail-tag">
              {who}
              {receipt.petName && receipt.handle ? ` · Tag #${receipt.handle}` : ""}
            </span>
          </div>
          <div className="kv">
            <span className="k">Purpose</span>
            <span className="v" data-testid="consent-detail-purpose-value">
              {receipt.purposeLabel ? `${humanizePurpose(receipt.purposeLabel)} (${receipt.purposeLabel})` : receipt.purposeHex}
            </span>
          </div>
          <div className="kv">
            <span className="k">Record type</span>
            <span className="v" data-testid="consent-detail-recordtype">
              {receipt.recordTypeLabel ?? receipt.recordTypeHex ?? "Unavailable (transaction unreadable)"}
            </span>
          </div>
          <div className="kv">
            <span className="k">Verified by (relayer)</span>
            <span className="v">
              <a
                href={`${ROAX_EXPLORER_URL}/address/${receipt.relayer}`}
                target="_blank"
                rel="noreferrer"
                data-testid="consent-detail-relayer"
              >
                {receipt.relayer}
              </a>
            </span>
          </div>
          <div className="kv">
            <span className="k">Consent window ends</span>
            <span className="v" data-testid="consent-detail-deadline">
              {fmtDateTime(receipt.deadline)}
            </span>
          </div>
        </div>

        <h2 style={{ marginTop: 20 }}>On-chain confirmation</h2>
        <div>
          <div className="kv">
            <span className="k">Recorded at</span>
            <span className="v" data-testid="consent-detail-granted">
              {fmtDateTime(receipt.grantedAt)} · block {receipt.blockNumber}
            </span>
          </div>
          <div className="kv">
            <span className="k">Transaction</span>
            <span className="v">
              <a
                href={`${ROAX_EXPLORER_URL}/tx/${receipt.txHash}`}
                target="_blank"
                rel="noreferrer"
                data-testid="consent-detail-tx"
              >
                {receipt.txHash}
              </a>
            </span>
          </div>
          <div className="kv">
            <span className="k">Consent nullifier</span>
            <span className="v" data-testid="consent-detail-nullifier">
              {receipt.nullifier}
            </span>
          </div>
          <div className="kv">
            <span className="k">Registry</span>
            <span className="v">
              {VERIFICATION_REGISTRY_CONSENT_ADDR} · ROAX · chainId {ROAX_CHAIN_ID}
            </span>
          </div>
        </div>

        <p className="sub" style={{ marginTop: 16 }}>
          The nullifier is this consent's one-time fingerprint: the registry consumes it on-chain, so
          the same signed grant can never be replayed. Neither it, the tag id, nor anything else in
          this record identifies you on-chain.
        </p>
      </div>
    </div>
  );
}
