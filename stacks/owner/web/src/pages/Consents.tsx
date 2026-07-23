import { Link } from "react-router-dom";
import { humanizePurpose, windowOpen, type ConsentReceipt } from "../lib/consents";
import { shortAddr, useConsentReceipts, useCredentials } from "../lib/hooks";

/**
 * CONSENT HISTORY (govarch PR-6, consent-receipt renderer) - the owner's own record of the consents
 * they granted: when, to whom (relayer), for what purpose/record type, with what window, and the
 * on-chain confirmation. Assembled ENTIRELY on this device from the held credentials' tag ids plus
 * the public owner-blind `Verified` events - a read-only RPC lookup, no backend, nothing shared.
 *
 * Each entry is a completed point-in-time act: the proof was approved once and its nullifier is
 * consumed on-chain. This page renders history; there is nothing here to cancel or undo.
 */

function fmtDateTime(unixSec: number): string {
  return new Date(unixSec * 1000).toLocaleString();
}

function fmtDate(unixSec: number): string {
  return new Date(unixSec * 1000).toLocaleDateString();
}

export function ConsentRow({ receipt }: { receipt: ConsentReceipt }) {
  const open = windowOpen(receipt);
  const who = receipt.petName ?? (receipt.handle ? `Tag #${receipt.handle}` : "Unknown tag");
  return (
    <Link
      to={`/consents/${encodeURIComponent(receipt.nullifier)}`}
      className="receipt-list-row"
      data-testid="consent-row"
    >
      <div>
        <div className="receipt-list-title" data-testid="consent-row-purpose">
          {receipt.purposeLabel ? humanizePurpose(receipt.purposeLabel) : shortAddr(receipt.purposeHex)}
        </div>
        <div className="receipt-list-meta">
          {who} · {receipt.recordTypeLabel ?? "—"} · to {shortAddr(receipt.relayer)} ·{" "}
          {fmtDateTime(receipt.grantedAt)}
        </div>
      </div>
      <div className="receipt-list-side">
        <span className={`badge ${open ? "ok" : ""}`} data-testid="consent-row-window">
          {open ? "Window open" : "Window closed"}
        </span>
        <span className="receipt-list-date">until {fmtDate(receipt.deadline)}</span>
      </div>
    </Link>
  );
}

/** The owner's consent history - every owner-blind `Verified` event on the held tags. */
export function Consents() {
  const credentials = useCredentials();
  const { state, receipts, reload } = useConsentReceipts(credentials);

  return (
    <>
      <div className="section-title">
        <h1>Consents</h1>
        <span className="count-pill" data-testid="consent-count">
          {state === "ready" ? `${receipts.length} granted` : "…"}
        </span>
      </div>

      {credentials.length === 0 ? (
        <div className="empty" data-testid="empty-consents-no-credentials">
          <div className="big" aria-hidden>
            🔏
          </div>
          <h3>No credentials to look up</h3>
          <p>
            Your consent history is derived from the dog-tag ids inside the credentials you hold.
            Receive a credential first, and the consents you granted for its tag will appear here.
          </p>
          <Link to="/receive" className="btn">
            Receive a credential
          </Link>
        </div>
      ) : state === "loading" ? (
        <div className="empty" data-testid="consents-loading">
          <div className="big" aria-hidden>
            ⏳
          </div>
          <h3>Reading ROAX…</h3>
          <p>Looking up the owner-blind verification events for your tags.</p>
        </div>
      ) : state === "error" ? (
        <div className="empty" data-testid="consents-error">
          <div className="big" aria-hidden>
            ⚠️
          </div>
          <h3>Could not reach ROAX</h3>
          <p>Your consent history lives on-chain; it will be here when the network is reachable.</p>
          <button type="button" className="btn" data-testid="consents-retry" onClick={reload}>
            Try again
          </button>
        </div>
      ) : receipts.length === 0 ? (
        <div className="empty" data-testid="empty-consents">
          <div className="big" aria-hidden>
            🔏
          </div>
          <h3>No consents recorded yet</h3>
          <p>
            When you approve a consent request on your device, the verification is recorded on-chain
            without your identity - and shows up here as a receipt.
          </p>
        </div>
      ) : (
        <div className="receipt-list" data-testid="consent-list">
          {receipts.map((r) => (
            <ConsentRow key={r.nullifier} receipt={r} />
          ))}
        </div>
      )}

      <div className="card">
        <h2>How this history works</h2>
        <p className="sub">
          Each entry is a point-in-time consent you approved on your device with a zero-knowledge
          proof. The chain records it owner-blind - your identity is never on-chain - so this page
          rebuilds your history locally from the tag ids in your held credentials plus the public
          verification events. Only read-only network lookups leave this device. A closed window is
          simply history: the grant was already used the moment it was recorded.
        </p>
      </div>
    </>
  );
}
