import { useEffect, useMemo, useState } from "react";
import { Link } from "react-router-dom";
import { CredentialCard } from "../components/CredentialCard";
import { isRootAnchored } from "../lib/chain";
import { summarize } from "../lib/credential";
import { useCredentials } from "../lib/hooks";
import {
  buildReceipt,
  deriveStatus,
  isReceiptType,
  statusBadgeClass,
  statusLabel,
  type OnChain,
} from "../lib/receipt";
import type { StoredCredential } from "../lib/store";

function ReceiptRow({ credential }: { credential: StoredCredential }) {
  const summary = summarize(credential.wrappedDoc);
  const receipt = useMemo(() => buildReceipt(credential.wrappedDoc), [credential.wrappedDoc]);
  const [onchain, setOnchain] = useState<OnChain>("checking");
  const validUntil = receipt?.validUntil ?? "";

  useEffect(() => {
    let live = true;
    setOnchain("checking");
    isRootAnchored(summary.documentStore, summary.credentialRoot)
      .then((ok) => live && setOnchain(ok ? "valid" : "invalid"))
      .catch(() => live && setOnchain("unknown"));
    return () => {
      live = false;
    };
  }, [summary.documentStore, summary.credentialRoot]);

  const status = deriveStatus(onchain, validUntil);

  if (!receipt) return null;

  return (
    <Link
      to={`/receipt/${encodeURIComponent(credential.id)}`}
      className="receipt-list-row"
      data-testid="receipt-row"
    >
      <div>
        <div className="receipt-list-title">{summary.petName ?? receipt.title}</div>
        <div className="receipt-list-meta">
          {summary.recordType} · Receipt {receipt.receiptId || "—"} · Dog tag #{receipt.dogTagId || "—"}
        </div>
      </div>
      <div className="receipt-list-side">
        <span className={`badge ${statusBadgeClass(status)}`} data-testid="receipt-row-status">
          {statusLabel(status)}
        </span>
        {receipt.validUntil && <span className="receipt-list-date">Valid until {receipt.validUntil}</span>}
      </div>
    </Link>
  );
}

/** Index of held credentials that can render as official receipt sheets. */
export function Receipts() {
  const credentials = useCredentials();
  const receiptCredentials = credentials.filter((c) => isReceiptType(summarize(c.wrappedDoc).recordType));

  return (
    <>
      <div className="section-title">
        <h1>Receipts</h1>
        <span className="count-pill" data-testid="receipt-count">
          {receiptCredentials.length} available
        </span>
      </div>

      {receiptCredentials.length === 0 ? (
        <div className="empty" data-testid="empty-receipts">
          <div className="big" aria-hidden>
            🧾
          </div>
          <h3>No receipt credentials yet</h3>
          <p>
            Travel clearances and government health certificates you receive will appear here as
            printable, phone-showable receipts.
          </p>
          <Link to="/receive" className="btn">
            Receive a credential
          </Link>
        </div>
      ) : (
        <div className="receipt-list" data-testid="receipt-list">
          {receiptCredentials.map((c) => (
            <ReceiptRow key={c.id} credential={c} />
          ))}
        </div>
      )}

      <div className="card">
        <h2>All held credentials</h2>
        <p className="sub">
          Receipts are derived from your local wrapped credential. No backend is used; the only live
          lookup is the ROAX validity read.
        </p>
        <div className="cred-list">
          {credentials.map((c) => (
            <CredentialCard key={c.id} credential={c} />
          ))}
        </div>
      </div>
    </>
  );
}
