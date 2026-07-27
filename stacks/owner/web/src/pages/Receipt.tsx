import { useEffect, useMemo, useState } from "react";
import { Link, useParams } from "react-router-dom";
import { QRCodeSVG } from "qrcode.react";
import { isRootAnchored } from "../lib/chain";
import { summarize } from "../lib/credential";
import { useCredentials } from "../lib/hooks";
import {
  buildReceipt,
  deriveStatus,
  statusAccent,
  statusLabel,
  type OnChain,
  type ReceiptSection,
} from "../lib/receipt";

/**
 * The holder's CDC-grade travel-clearance RECEIPT — the "show this on your phone" surface (govarch
 * PR-6). It renders the held TRAVEL_CLEARANCE / EU_HEALTH_CERT credential as the official CDC "Dog
 * Import Form Receipt" anatomy: letterhead, a derived VALID / EXPIRED / REVOKED status banner,
 * the Receipt ID / issuance / validity identity block, the legal preamble, Section A/B/C tables and
 * a Verification block whose QR points at the PII-free public status page `/r/:receiptId`.
 *
 * It is built ENTIRELY from the locally-held WrappedDoc (decoded Merkle leaves) plus one live on-chain
 * `DogTagIssuer.isValid(root)` read — no backend, offline-capable except the status read (govarch-r8
 * §3.2). A received *redacted* copy keeps the on-chain root but drops the withheld leaves; those rows
 * are omitted and the count is flagged, so what the holder disclosed is exactly what the receipt shows.
 */

/** A CDC section: a dark-slate header bar + a bordered label/value table (blank rows already dropped). */
function Section({ section }: { section: ReceiptSection }) {
  return (
    <div className="receipt-section">
      <div className="receipt-section-bar">{section.title}</div>
      <table className="receipt-table">
        <tbody>
          {section.rows.map((r) => (
            <tr key={r.label}>
              <td className="r-label">{r.label}</td>
              <td className="r-value">{r.value}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

export function Receipt() {
  const { id = "" } = useParams();
  const decodedId = decodeURIComponent(id);
  const credentials = useCredentials();
  const credential = credentials.find((c) => c.id === decodedId);
  const doc = credential?.wrappedDoc;

  const [onchain, setOnchain] = useState<OnChain>("checking");

  useEffect(() => {
    if (!doc) return;
    let live = true;
    setOnchain("checking");
    isRootAnchored(doc.issuer.documentStore, doc.signature.merkleRoot)
      .then((ok) => live && setOnchain(ok ? "valid" : "invalid"))
      .catch(() => live && setOnchain("unknown"));
    return () => {
      live = false;
    };
  }, [doc?.signature.merkleRoot, doc?.issuer.documentStore]);

  const receipt = useMemo(() => (doc ? buildReceipt(doc) : null), [doc]);

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

  if (!receipt) {
    const summary = summarize(doc);
    return (
      <div className="card" data-testid="receipt-unavailable">
        <h2>Receipt unavailable</h2>
        <p className="sub">
          {summary.recordType} credentials do not have an official holder receipt view. Open the
          credential detail to inspect its decoded Merkle leaves or share a redacted copy.
        </p>
        <div className="btn-row">
          <Link to={`/credential/${encodeURIComponent(credential.id)}`} className="btn">
            Back to credential
          </Link>
          <Link to={`/share/${encodeURIComponent(credential.id)}`} className="btn secondary">
            Share a redacted copy →
          </Link>
        </div>
      </div>
    );
  }

  const status = deriveStatus(onchain, receipt.validUntil);
  const accent = statusAccent(status);

  const liveText =
    onchain === "valid"
      ? "On-chain: anchored & valid"
      : onchain === "invalid"
        ? "On-chain: revoked or not anchored"
        : onchain === "checking"
          ? "On-chain: checking…"
          : "On-chain status unconfirmed (chain unreachable)";
  const liveDot =
    onchain === "valid" ? "#16a34a" : onchain === "invalid" ? "#dc2626" : "#64748b";

  return (
    <div>
      {/* controls — never printed */}
      <div className="no-print receipt-controls">
        <Link to={`/credential/${encodeURIComponent(credential.id)}`} className="back-link" style={{ margin: 0 }}>
          ← Back to credential
        </Link>
        <div className="btn-row" style={{ marginTop: 0 }}>
          <Link
            to={`/share/${encodeURIComponent(credential.id)}`}
            className="btn secondary"
            data-testid="receipt-share"
          >
            Share a redacted copy →
          </Link>
          <button
            type="button"
            className="btn"
            data-testid="receipt-print"
            onClick={() => window.print()}
          >
            🖨 Print receipt
          </button>
        </div>
      </div>

      <div className="receipt-sheet" data-testid="receipt-sheet">
        {/* letterhead */}
        <div className="receipt-letterhead">
          <span className="receipt-seal" aria-hidden>
            🏛
          </span>
          <div>
            <div className="receipt-authority" data-testid="receipt-authority">
              {receipt.authorityName}
            </div>
            <div className="receipt-subtitle">
              {receipt.isTravel
                ? "Pet Travel Clearance · authority-endorsed credential"
                : "Pet Health Certificate · authority-endorsed credential"}
            </div>
          </div>
        </div>

        <div className="receipt-body">
          {/* title + derived status */}
          <div className="receipt-title-row">
            <h2 className="receipt-title">{receipt.title}</h2>
            <span className="receipt-status-chip" style={{ background: accent }} data-testid="receipt-status">
              ● {statusLabel(status).toUpperCase()}
            </span>
          </div>

          {/* identity block */}
          <div className="receipt-id-block" data-testid="receipt-id">
            Receipt ID: {receipt.receiptId || "—"}
          </div>
          <table className="receipt-table receipt-identity">
            <tbody>
              <tr>
                <td className="r-label">Date of issuance</td>
                <td className="r-value">{receipt.issuanceDate || "—"}</td>
              </tr>
              <tr>
                <td className="r-label">
                  {receipt.multipleEntries ? "Valid for multiple entries until" : "Valid until"}
                </td>
                <td className="r-value">{receipt.validUntil || "—"}</td>
              </tr>
              <tr>
                <td className="r-label">Dog Tag ID (SBT)</td>
                <td className="r-value">#{receipt.dogTagId || "—"}</td>
              </tr>
            </tbody>
          </table>

          {/* legal preamble */}
          {receipt.isTravel ? (
            <p className="receipt-preamble">
              This receipt is valid for the animal listed for the validity window shown above
              {receipt.departureBinding
                ? `, for entry from the listed country of departure (${receipt.departureBinding})`
                : ""}
              . If the animal travels via a different or high-risk country, a new clearance may be
              required. <strong>You must show this receipt (printed or on your phone) to airline
              staff and port-of-entry officials.</strong> The authority reserves the right to request
              additional supporting documentation on arrival.
            </p>
          ) : (
            <p className="receipt-preamble">
              This certificate is valid for the animal listed for the validity window shown above.
              <strong> You must show this certificate when a verifier or competent authority requests
              the animal's health credential.</strong> The authority reserves the right to request
              additional supporting documentation.
            </p>
          )}

          {receipt.obfuscatedCount > 0 && (
            <p className="receipt-withheld-note" data-testid="receipt-withheld-note">
              Note: {receipt.obfuscatedCount} field(s) were withheld (obfuscated) by the holder in this
              copy; withheld leaves still verify against the on-chain root.
            </p>
          )}

          {/* sectioned CDC content */}
          {receipt.sections.map((section) => (
            <Section key={section.title} section={section} />
          ))}

          {/* verification block — QR to the PII-free public status page + on-chain provenance */}
          <div className="receipt-section">
            <div className="receipt-section-bar">Verification</div>
            <div className="receipt-verify" data-testid="receipt-verify">
              {receipt.publicStatusUrl && (
                <div className="receipt-qr" data-testid="receipt-qr">
                  <QRCodeSVG value={receipt.publicStatusUrl} size={132} level="M" />
                </div>
              )}
              <div className="receipt-verify-body">
                <div className="receipt-verify-title">
                  {receipt.publicStatusUrl
                    ? "Scan to confirm live status on-chain"
                    : "No public status page for this receipt"}
                </div>
                <div className="receipt-verify-url" data-testid="receipt-public-url">
                  {receipt.publicStatusUrl ||
                    "This credential's issuer published no reachable status URL. Confirm it against the on-chain record below."}
                </div>

                <div className="receipt-live" data-testid="receipt-live">
                  <span className="receipt-live-dot" style={{ background: liveDot }} />
                  {liveText}
                </div>

                <div className="receipt-provenance">
                  <span className="k">Credential root</span>
                  <div className="receipt-mono" data-testid="receipt-root">
                    {receipt.root}
                  </div>
                </div>
                <div className="receipt-provenance-line">
                  Anchored: ROAX · chainId 135 · issuer{" "}
                  <span className="receipt-mono">{receipt.documentStore.slice(0, 12)}…</span>
                </div>
                {receipt.legalBasisVersion && (
                  <div className="receipt-provenance-line">
                    Legal basis: {receipt.legalBasisVersion} · authority endorsement
                  </div>
                )}
                <div className="receipt-verify-note">
                  The public status page shows NO personal data — only the live VALID / EXPIRED /
                  REVOKED verdict and provenance.
                </div>
              </div>
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}
